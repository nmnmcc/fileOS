// SPDX-License-Identifier: AGPL-3.0-only

//! The homeserver binary: one program, one data directory.
//!
//! Keep this crate thin. It wires the components together and nothing else,
//! so that editing it rebuilds in seconds.

use std::io::{BufRead, IsTerminal, Read, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use clap::{Args, Parser, Subcommand, ValueEnum};
use hs_api::Api;
use hs_api::replica::{self, Store};
use hs_node::Node;
use hs_space::Role;
use serde_json::Value as Json;
use tokio::net::TcpListener;
use tokio::sync::watch;

mod commands;
mod output;
mod remote;

use commands::Global;
use output::print_json;

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

const COMPONENTS: &[&str] = &[
    hs_core::NAME,
    hs_identity::NAME,
    hs_store::NAME,
    hs_net::NAME,
    hs_node::NAME,
    hs_api::NAME,
    hs_plugin_api::NAME,
    hs_plugin_host::NAME,
];

const LETS_ENCRYPT: &str = "https://acme-v02.api.letsencrypt.org/directory";
const RENEW_AFTER: Duration = Duration::from_hours(1440);

const LONG_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "\nLicense AGPL-3.0-only: GNU AGPL version 3 <https://www.gnu.org/licenses/agpl-3.0.html>",
    "\nhomeserver is free software and comes with ABSOLUTELY NO WARRANTY.",
    "\nThis software is based in part on the work of the Independent JPEG Group."
);

type Res<T = ()> = Result<T, String>;

/// homeserver: your own server for the things you keep and share.
///
/// `hs` looks after a node on this machine (init, serve, config, …) and
/// works with a running one over its API, here or elsewhere, once signed in
/// with `hs login URL`. Commands that can do both use the data directory
/// while the node is stopped, and its API while it runs.
#[derive(Parser)]
#[command(name = "hs", version, long_version = LONG_VERSION, arg_required_else_help = true)]
struct Cli {
    /// The data directory [default: $`XDG_DATA_HOME/homeserver`]
    #[arg(long, env = "HS_DIR", global = true, value_name = "DIR")]
    dir: Option<PathBuf>,

    /// The node to talk to [default: the one last signed in to]
    #[arg(long, env = "HS_SERVER", global = true, value_name = "URL")]
    server: Option<String>,

    /// Print what the API returns, as JSON
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a node; prints the recovery code once
    Init {
        /// Your name, shown to the people you share with
        #[arg(long)]
        name: String,
        /// The URL other nodes reach this one at
        #[arg(long)]
        url: Option<String>,
    },
    /// Run the node
    Serve(ServeArgs),
    /// Sign in to a running node with a token (read from the terminal or stdin)
    Login {
        /// Its address, such as hs.example.org [default: 127.0.0.1:8080]
        url: Option<String>,
        /// Accept a certificate nobody vouches for (from `hs serve --tls self-signed`)
        #[arg(long)]
        insecure: bool,
    },
    /// Forget the sign-in to a node [default: the current one]
    Logout { url: Option<String> },
    /// Show the node signed in to, and how it is doing
    Status,
    /// List resources in a space
    Ls(commands::LsArgs),
    /// Print a resource
    Get(commands::GetArgs),
    /// Create or replace a resource from a file or stdin
    Put(commands::PutArgs),
    /// Change a resource in $EDITOR
    Edit(commands::EditArgs),
    /// Change some fields of a JSON resource
    Patch(commands::PatchArgs),
    /// Delete a resource
    Rm(commands::RmArgs),
    /// Upload files into a space
    Upload(commands::UploadArgs),
    /// Search everything you can read
    Search(commands::SearchArgs),
    /// The newest things first
    Timeline(commands::TimelineArgs),
    /// Things that happened near a place
    Near(commands::NearArgs),
    /// What links to or from a resource
    Related(commands::RelatedArgs),
    /// Raw bytes, by hash
    #[command(subcommand)]
    Blob(commands::BlobCommand),
    /// Print changes as they happen
    Watch,
    /// Tokens for apps and people
    #[command(subcommand)]
    Token(commands::TokenCommand),
    /// Spaces and their members
    #[command(subcommand)]
    Space(commands::SpaceCommand),
    /// Print the DID document of your identity
    Identity,
    /// Split the recovery code among friends, or put it back together
    #[command(subcommand)]
    Recovery(RecoveryCommand),
    /// Read or change the configuration
    #[command(subcommand)]
    Config(ConfigCommand),
    /// Write a plain tar archive of everything to stdout
    Export,
    /// Encrypted copies of the data directory elsewhere
    #[command(subcommand)]
    Replica(commands::ReplicaCommand),
    /// Bring a node back from a replica
    ///
    /// The recovery code is read from $`HS_RECOVERY_CODE`, else from stdin.
    Restore {
        /// The replica: a directory, s3://… or https://…
        #[arg(long, value_name = "REPLICA")]
        from: String,
    },
    /// Rebuild the derived index from the logs and blobs
    RebuildIndex,
    /// List the components built into this executable
    #[command(hide = true)]
    Components,
    #[command(hide = true)]
    ThumbnailWorker { side: u32 },
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Tls {
    /// Plain HTTP
    Off,
    /// A certificate made and signed by this node
    SelfSigned,
    /// A certificate from an ACME authority such as Let's Encrypt (HTTP-01)
    Acme,
}

#[derive(Args)]
struct ServeArgs {
    /// Listen address. Defaults to `127.0.0.1:8080`, or `[::]:443` with `--tls acme`.
    #[arg(long, value_name = "ADDR")]
    listen: Option<SocketAddr>,
    #[arg(long, value_enum, default_value_t = Tls::Off)]
    tls: Tls,
    /// Domain names for the certificate [default: the host of the configured url]
    #[arg(long = "domain", value_name = "DOMAIN")]
    domains: Vec<String>,
    /// Contact address for the ACME account
    #[arg(long)]
    email: Option<String>,
    /// ACME directory URL
    #[arg(long, value_name = "URL", default_value = LETS_ENCRYPT)]
    acme_directory: String,
    /// Plain HTTP address for ACME challenges, with --tls acme
    #[arg(long, value_name = "ADDR", default_value = "[::]:80")]
    http_listen: SocketAddr,
    /// Ask the home router to forward the port (NAT-PMP/PCP)
    #[arg(long)]
    map_port: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum RoleArg {
    Reader,
    Writer,
    Owner,
}

impl From<RoleArg> for Role {
    fn from(r: RoleArg) -> Role {
        match r {
            RoleArg::Reader => Role::Reader,
            RoleArg::Writer => Role::Writer,
            RoleArg::Owner => Role::Owner,
        }
    }
}

#[derive(Subcommand)]
enum RecoveryCommand {
    /// Split the recovery code (read from stdin) into shares
    Split {
        /// How many shares restore the code
        #[arg(long, value_name = "K")]
        threshold: u8,
        /// How many shares to make
        #[arg(long, value_name = "N")]
        count: u8,
    },
    /// Combine shares (on stdin, one per line) into the recovery code
    Combine,
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Print the configuration, or one key of it
    Get { key: Option<String> },
    /// Set a key; the value is JSON, or else a string
    Set { key: String, value: String },
}

pub(crate) fn data_dir(dir: Option<PathBuf>) -> PathBuf {
    if let Some(d) = dir {
        return d;
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| ".".into()))
                .join(".local/share")
        });
    base.join("homeserver")
}

pub(crate) async fn open(dir: &Path) -> Res<Node> {
    if !dir.join("config.json").exists() {
        return Err(format!(
            "{} holds no node; run `hs init --name NAME` first, or sign in to a running one with `hs login URL`",
            dir.display()
        ));
    }
    Node::open(dir).await.map_err(|e| {
        let e = e.to_string();
        if e.contains("another hs process") {
            format!("{e}; stop it first, or do this through its API (`hs login`)")
        } else {
            e
        }
    })
}

fn stdin_line(prompt: &str) -> Res<String> {
    if std::io::stdin().is_terminal() {
        eprint!("{prompt}");
        let _ = std::io::stderr().flush();
    }
    let mut s = String::new();
    std::io::stdin()
        .read_line(&mut s)
        .map_err(|e| e.to_string())?;
    Ok(s.trim().to_string())
}

fn recovery_seed(input: &str) -> Res<[u8; 32]> {
    hs_identity::parse_recovery_code(input)
        .ok_or_else(|| "that is not a valid recovery code".to_string())
}

// init

async fn init(dir: &Path, name: &str, url: Option<&str>) -> Res {
    let done = Node::init(dir, name, url)
        .await
        .map_err(|e| e.to_string())?;
    let id = done.node.config().identity.to_id();
    drop(done.node);
    println!("Created a node in {}", dir.display());
    println!("Identity: {id}\n");
    println!(
        "Recovery code (write it down; it is not stored anywhere):\n\n    {}\n",
        done.recovery_code
    );
    println!(
        "Owner token for your first browser or app:\n\n    {}\n",
        done.owner_token
    );
    // The first node made here is the one `hs` talks to once it runs.
    if !remote::has_logins()? {
        let url = match url {
            Some(u) => remote::normalize_url(u)?,
            None => remote::LOCAL_URL.to_string(),
        };
        remote::save_login(&url, &done.owner_token, false)?;
        println!("hs is signed in to {url} with that token (change it with hs login).");
    }
    println!("Next: hs serve");
    Ok(())
}

// serve

fn stop_signal() -> watch::Receiver<bool> {
    let (tx, rx) = watch::channel(false);
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = tx.send(true);
    });
    rx
}

async fn stopped(mut rx: watch::Receiver<bool>) {
    let _ = rx.wait_for(|v| *v).await;
}

fn bind(a: SocketAddr) -> Res<std::net::TcpListener> {
    let l = std::net::TcpListener::bind(a).map_err(|e| format!("cannot listen on {a}: {e}"))?;
    l.set_nonblocking(true).map_err(|e| e.to_string())?;
    Ok(l)
}

fn tokio_listener(l: &std::net::TcpListener) -> Res<TcpListener> {
    TcpListener::from_std(l.try_clone().map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// Ask the router to forward `port`, and keep renewing the mapping.
fn map_port(port: u16) {
    tokio::spawn(async move {
        let Some(gw) = hs_net::natpmp::default_gateway() else {
            tracing::warn!(name: "port.mapping.no_gateway", port, "No gateway found for port {{port}}");
            return;
        };
        let mut m = match hs_net::natpmp::map_tcp(gw, port, port, 7200).await {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(name: "port.mapping.failed", port, error = %e, "Could not map port {{port}}");
                return;
            }
        };
        tracing::info!(name: "port.mapping.started", port, external_port = m.external_port,
            "Forwarded external port {{external_port}} to {{port}}");
        loop {
            tokio::time::sleep(Duration::from_secs(u64::from(m.lifetime / 2).max(60))).await;
            match hs_net::natpmp::renew_tcp(gw, &m, 7200).await {
                Ok(n) => m = n,
                Err(error) => {
                    tracing::warn!(name: "port.mapping.renewal_failed", port, %error, "Could not renew the mapping for port {{port}}");
                }
            }
        }
    });
}

fn domains(api: &Api, args: &ServeArgs) -> Vec<String> {
    let mut d = args.domains.clone();
    if d.is_empty()
        && let Some(url) = api.node.config().url
        && let Ok(uri) = url.parse::<http::Uri>()
        && let Some(h) = uri.host()
    {
        d.push(h.to_string());
    }
    d
}

fn read_pem(dir: &Path, name: &str) -> Option<(String, String, SystemTime)> {
    let cert = dir.join(format!("keys/{name}.pem"));
    let modified = std::fs::metadata(&cert).and_then(|m| m.modified()).ok()?;
    Some((
        std::fs::read_to_string(cert).ok()?,
        std::fs::read_to_string(dir.join(format!("keys/{name}.key"))).ok()?,
        modified,
    ))
}

fn write_pem(dir: &Path, name: &str, cert: &str, key: &str) -> Res {
    std::fs::write(dir.join(format!("keys/{name}.pem")), cert).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(format!("keys/{name}.key")), key).map_err(|e| e.to_string())
}

async fn acme_certificate(api: &Api, args: &ServeArgs, names: &[String]) -> Res<(String, String)> {
    let dir = api.node.dir();
    let account_path = dir.join("keys/acme-account");
    let account = std::fs::read(&account_path).ok();
    let acme =
        hs_net::acme::Acme::new(api.web.clone(), &args.acme_directory, account).map_err(|e| e.0)?;
    std::fs::write(&account_path, acme.account_key_pkcs8()).map_err(|e| e.to_string())?;
    let contact = args.email.as_ref().map(|e| format!("mailto:{e}"));
    let (cert, key) = acme
        .obtain(names, contact.as_deref(), &api.acme)
        .await
        .map_err(|e| e.0)?;
    write_pem(dir, "acme", &cert, &key)?;
    Ok((cert, key))
}

async fn serve(dir: &Path, args: &ServeArgs) -> Res {
    let node = Arc::new(open(dir).await?);
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let api = Api::with_thumbnail_worker(Arc::clone(&node), executable);
    api.start();
    let listen = args.listen.unwrap_or_else(|| {
        if args.tls == Tls::Acme {
            SocketAddr::from(([0u16; 8], 443))
        } else {
            SocketAddr::from(([127, 0, 0, 1], 8080))
        }
    });
    let listener = bind(listen)?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let stop = stop_signal();
    if args.map_port {
        map_port(port);
    }
    let scheme = if args.tls == Tls::Off {
        "http"
    } else {
        "https"
    };
    tracing::info!(name: "server.started", version = env!("CARGO_PKG_VERSION"), scheme, port,
        "Serving {{scheme}} on port {{port}}");
    match args.tls {
        Tls::Off => {
            hs_net::server::serve(
                tokio_listener(&listener)?,
                Arc::clone(&api),
                None,
                stopped(stop),
            )
            .await;
        }
        Tls::SelfSigned => {
            let (cert, key) = if let Some((c, k, _)) = read_pem(dir, "self-signed") {
                (c, k)
            } else {
                let mut names = domains(&api, args);
                names.extend(["localhost".to_string(), "127.0.0.1".to_string()]);
                let (c, k) = hs_net::server::self_signed(&names)?;
                write_pem(dir, "self-signed", &c, &k)?;
                (c, k)
            };
            let tls = hs_net::server::tls_config(&cert, &key)?;
            hs_net::server::serve(
                tokio_listener(&listener)?,
                Arc::clone(&api),
                Some(tls),
                stopped(stop),
            )
            .await;
        }
        Tls::Acme => {
            let names = domains(&api, args);
            if names.is_empty() {
                return Err("--tls acme needs --domain, or a url in the config".into());
            }
            // HTTP-01 challenges, and everything else, on plain HTTP too.
            let http = bind(args.http_listen)?;
            if args.map_port {
                map_port(http.local_addr().map_err(|e| e.to_string())?.port());
            }
            tokio::spawn(hs_net::server::serve(
                tokio_listener(&http)?,
                Arc::clone(&api),
                None,
                stopped(stop.clone()),
            ));
            loop {
                let (cert, key) = match read_pem(dir, "acme") {
                    Some((c, k, t)) if t.elapsed().unwrap_or_default() < RENEW_AFTER => (c, k),
                    old => match acme_certificate(&api, args, &names).await {
                        Ok(ck) => ck,
                        Err(e) => match old {
                            Some((c, k, _)) => {
                                tracing::warn!(name: "tls.certificate.renewal_failed", error = %e,
                                    "Could not renew the certificate, keeping the previous one");
                                (c, k)
                            }
                            None => return Err(format!("getting a certificate failed: {e}")),
                        },
                    },
                };
                let tls = hs_net::server::tls_config(&cert, &key)?;
                // Serve until a day passes (then check the certificate's age) or we are stopped.
                let mut s = stop.clone();
                let until = async move {
                    tokio::select! {
                        () = tokio::time::sleep(Duration::from_hours(24)) => {}
                        _ = s.wait_for(|v| *v) => {}
                    }
                };
                hs_net::server::serve(
                    tokio_listener(&listener)?,
                    Arc::clone(&api),
                    Some(tls),
                    until,
                )
                .await;
                if *stop.borrow() {
                    break;
                }
            }
        }
    }
    api.flush_access_log().await;
    node.index.close().await;
    tracing::info!(name: "server.stopped", "Server stopped");
    Ok(())
}

// recovery

/// Shares print as `hs-share-<x>-<check>-<base32 y>`, where `check` is a few
/// bytes of the secret's hash, so combining too few shares is noticed.
fn recovery(cmd: RecoveryCommand) -> Res {
    match cmd {
        RecoveryCommand::Split {
            threshold: k,
            count: n,
        } => {
            let seed = recovery_seed(&stdin_line("Recovery code: ")?)?;
            let mut random = Vec::new();
            for _ in 1..k {
                random.extend_from_slice(&hs_node::random::<32>());
            }
            let shares = hs_identity::shamir::split(&seed, k, n, &random)
                .ok_or("need 1 <= threshold <= count")?;
            let check = &hs_core::Hash::of(&seed).to_hex()[..8];
            for s in shares {
                println!("hs-share-{}-{check}-{}", s.x, hs_core::base32::encode(&s.y));
            }
            eprintln!(
                "Give one share to each person. Any {k} of the {n} restore the recovery code."
            );
            Ok(())
        }
        RecoveryCommand::Combine => {
            let mut shares = Vec::new();
            let mut check = None;
            for line in std::io::stdin().lock().lines() {
                let line = line.map_err(|e| e.to_string())?;
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let bad = || format!("not a share: {line}");
                let rest = line.strip_prefix("hs-share-").ok_or_else(bad)?;
                let mut parts = rest.splitn(3, '-');
                let x = parts.next().and_then(|x| x.parse().ok()).ok_or_else(bad)?;
                let c = parts.next().ok_or_else(bad)?.to_string();
                let y = parts
                    .next()
                    .and_then(hs_core::base32::decode)
                    .ok_or_else(bad)?;
                if check.as_ref().is_some_and(|k| *k != c) {
                    return Err("these shares come from different recovery codes".into());
                }
                check = Some(c);
                shares.push(hs_identity::shamir::Share { x, y });
            }
            let secret =
                hs_identity::shamir::combine(&shares).ok_or("could not combine these shares")?;
            let seed: [u8; 32] = secret
                .try_into()
                .map_err(|_| "shares have the wrong length")?;
            if Some(hs_core::Hash::of(&seed).to_hex()[..8].to_string()) != check {
                return Err("not enough shares (or a damaged one)".into());
            }
            println!("{}", hs_identity::recovery_code(&seed));
            Ok(())
        }
    }
}

// config, restore

async fn config(dir: &Path, cmd: ConfigCommand) -> Res {
    let node = open(dir).await?;
    match cmd {
        ConfigCommand::Get { key: None } => print_json(&node.config().raw),
        ConfigCommand::Get { key: Some(k) } => {
            print_json(node.config().raw.get(&k).unwrap_or(&Json::Null));
        }
        ConfigCommand::Set { key, value } => {
            let value = serde_json::from_str(&value).unwrap_or(Json::String(value));
            node.set_config(&key, value).map_err(|e| e.to_string())?;
        }
    }
    node.index.close().await;
    Ok(())
}

async fn restore(dir: &Path, from: &str) -> Res {
    if dir.join("config.json").exists() {
        return Err(format!("{} already holds a node", dir.display()));
    }
    let code = match std::env::var("HS_RECOVERY_CODE") {
        Ok(c) => c,
        Err(_) => stdin_line("Recovery code: ")?,
    };
    let seed = recovery_seed(&code)?;
    let store = Store::parse(from)?;
    let n = replica::restore(&store, hs_node::storage_key(&seed), dir).await?;
    eprintln!(
        "restored {n} files into {}; rebuilding the index",
        dir.display()
    );
    Node::rebuild_index(dir).await.map_err(|e| e.to_string())?;
    eprintln!("done; start it with hs serve");
    Ok(())
}

async fn run(g: Global, command: Command) -> Res {
    let dir = &data_dir(g.dir.clone());
    let g = &g;
    match command {
        Command::Login { url, insecure } => commands::login(url, insecure).await,
        Command::Logout { url } => commands::logout(url),
        Command::Status => commands::status(g).await,
        Command::Ls(a) => commands::ls(g, a).await,
        Command::Get(a) => commands::get(g, a).await,
        Command::Put(a) => commands::put(g, a).await,
        Command::Edit(a) => commands::edit(g, a).await,
        Command::Patch(a) => commands::patch(g, a).await,
        Command::Rm(a) => commands::rm(g, a).await,
        Command::Upload(a) => commands::upload(g, a).await,
        Command::Search(a) => commands::search(g, a).await,
        Command::Timeline(a) => commands::timeline(g, a).await,
        Command::Near(a) => commands::near(g, a).await,
        Command::Related(a) => commands::related(g, a).await,
        Command::Blob(cmd) => commands::blob(g, cmd).await,
        Command::Watch => commands::watch(g).await,
        Command::Init { name, url } => init(dir, &name, url.as_deref()).await,
        Command::Serve(args) => serve(dir, &args).await,
        Command::Token(cmd) => commands::token(g, cmd).await,
        Command::Space(cmd) => commands::space(g, cmd).await,
        Command::Identity => commands::identity(g).await,
        Command::Recovery(cmd) => recovery(cmd),
        Command::Config(cmd) => config(dir, cmd).await,
        Command::Export => commands::export(g).await,
        Command::Replica(cmd) => commands::replica(g, cmd).await,
        Command::Restore { from } => restore(dir, &from).await,
        Command::RebuildIndex => Node::rebuild_index(dir).await.map_err(|e| e.to_string()),
        Command::ThumbnailWorker { .. } => {
            unreachable!("thumbnail worker runs before starting Tokio")
        }
        Command::Components => {
            for name in COMPONENTS {
                println!("{name}");
            }
            Ok(())
        }
    }
}

fn thumbnail_worker(side: u32) -> std::io::Result<bool> {
    let limit = hs_net::server::MAX_BODY as u64;
    let mut input = Vec::new();
    std::io::stdin()
        .lock()
        .take(limit + 1)
        .read_to_end(&mut input)?;
    if input.len() as u64 > limit || !(1..=1024).contains(&side) {
        return Ok(false);
    }
    let Some((bytes, media_type)) = hs_media::thumbnail(&input, side) else {
        return Ok(false);
    };
    let tag = if media_type == "image/png" {
        b'p'
    } else {
        b'j'
    };
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&[tag])?;
    stdout.write_all(&bytes)?;
    stdout.flush()?;
    Ok(true)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Command::ThumbnailWorker { side } = cli.command {
        return match thumbnail_worker(side) {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::from(2),
            Err(_) => ExitCode::FAILURE,
        };
    }
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    let global = Global {
        dir: cli.dir,
        server: cli.server,
        json: cli.json,
    };
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("hs: {e}");
            return ExitCode::FAILURE;
        }
    };
    match rt.block_on(run(global, cli.command)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("hs: {e}");
            ExitCode::FAILURE
        }
    }
}
