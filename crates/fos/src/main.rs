// SPDX-License-Identifier: AGPL-3.0-only

//! `fos`: run a fileOS node, and use its kernel from the command line.
//!
//! The commands are in groups: `node`, `fs`, `space`, `identity` and `app`,
//! and `fos curl` runs curl against the gateway.
//! They are about the kernel only: what an app or a plugin does is not here,
//! but packing and installing them is. `fos node init` and `fos node run`
//! work on the data directory, and `fos app pack` on the files it is given.
//! The other commands talk to a running node through its gateway on this
//! machine.
//!
//! The command line keeps no state: it saves nothing of its own, and there
//! is no login. It reads the address and the token of the gateway from the
//! data directory of the node.

use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use fos_core::Error;
use fos_node::{NetConfig, Node};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};

#[derive(Parser)]
#[command(
    name = "fos",
    version,
    about = "fileOS: a file system that syncs between your nodes"
)]
struct Cli {
    /// The data directory of the node.
    #[arg(long, env = "FOS_DIR", default_value_os_t = default_dir(), global = true)]
    dir: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// This node: make it, run it, see what it does, connect it to others.
    Node {
        #[command(subcommand)]
        command: NodeCommand,
    },
    /// Files, by path.
    Fs {
        #[command(subcommand)]
        command: FsCommand,
    },
    /// Spaces: make them, mount them, give and take back access.
    Space {
        #[command(subcommand)]
        command: SpaceCommand,
    },
    /// The identity (a did:fos DID) that this node is a node of. With no
    /// command, show it.
    #[command(visible_alias = "id")]
    Identity {
        #[command(subcommand)]
        command: Option<IdentityCommand>,
    },
    /// Apps: pack them, install them.
    App {
        #[command(subcommand)]
        command: AppCommand,
    },
    /// Run curl with this node's token, against its gateway. Every
    /// connection goes to the gateway, so the token never leaves this
    /// machine. A last argument that is only a path, such as /v0/status, is
    /// a request to the kernel API.
    Curl {
        /// The arguments for curl.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

#[derive(Subcommand)]
enum NodeCommand {
    /// Make a new node, with its home space.
    Init,
    /// Run the node: the network, the apps and the gateway.
    Run {
        /// libp2p addresses to listen on.
        #[arg(long, default_values = ["/ip4/0.0.0.0/tcp/0", "/ip6/::/tcp/0"])]
        listen: Vec<String>,
        /// Nodes to connect to at the start.
        #[arg(long)]
        peer: Vec<String>,
        /// The port of the gateway, on this machine only. 0 is any free port.
        #[arg(long, default_value_t = fos_gateway::DEFAULT_PORT)]
        port: u16,
        /// Do not look for nodes on the home network.
        #[arg(long)]
        no_mdns: bool,
    },
    /// What the node is doing: its key, addresses, peers and spaces.
    Status,
    /// Connect to another node by its multiaddr.
    Connect { addr: String },
}

#[derive(Subcommand)]
enum FsCommand {
    /// The names under a path.
    Ls {
        #[arg(default_value = "/")]
        path: String,
    },
    /// Print a file.
    Cat { path: String },
    /// Put a file. With no FILE, it reads standard input.
    Put { path: String, file: Option<PathBuf> },
    /// Add a line to the end of a file.
    Append { path: String, text: String },
    /// Take a file or mount away. Old versions stay in the history.
    Rm { path: String },
}

#[derive(Subcommand)]
enum SpaceCommand {
    /// Make a space and mount it at a path. Prints its ID.
    New { at: String },
    /// Mount a space from other nodes at a path.
    Mount { id: String, at: String },
    /// Give a key permissions on a path: on the file there, or on every
    /// name under it. PERMISSIONS are exact names (files.read, access.give,
    /// and so on) or templates (read, read-write, manage, relay), with
    /// commas. Prints the ID of the grant.
    Grant {
        path: String,
        key: String,
        permissions: String,
    },
    /// The grants in the space that has a path.
    Grants {
        #[arg(default_value = "/")]
        path: String,
    },
    /// Take back a grant, by its ID, in the space that has a path. What was
    /// given on from it is taken back too.
    Take { path: String, id: String },
    /// Make a checkpoint of the space that has a path, and drop the entries
    /// before it.
    Checkpoint { path: String },
}

#[derive(Subcommand)]
enum IdentityCommand {
    /// Make a new identity. This node holds its keys and is its first node.
    New,
    /// Add a node to the identity. Only the node that holds the signing key can.
    AddNode { key: String },
    /// Take a node away from the identity.
    RemoveNode { key: String },
    /// Join an identity, with the space ID that `fos identity add-node` gave.
    Join { space: String },
}

#[derive(Subcommand)]
enum AppCommand {
    /// Install an app from an .app file (or an .app folder, while you make it).
    Install { app: PathBuf },
    /// Pack an .app folder into an .app file. Works with no running node.
    Pack {
        /// The .app folder.
        folder: PathBuf,
        /// The .app file to make. It must not be there yet.
        file: PathBuf,
    },
}

fn default_dir() -> PathBuf {
    std::env::var_os("HOME").map_or_else(
        || PathBuf::from(".fileos"),
        |home| PathBuf::from(home).join(".fileos"),
    )
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,libp2p=warn")),
        )
        .with_writer(std::io::stderr)
        .init();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("fos: {e}");
            ExitCode::FAILURE
        }
    }
}

fn enc(text: &str) -> String {
    utf8_percent_encode(text, NON_ALPHANUMERIC).to_string()
}

/// A node path in a URL: each part encoded, the slashes kept.
fn url_path(path: &str) -> String {
    path.split('/').map(enc).collect::<Vec<_>>().join("/")
}

fn run(cli: Cli) -> Result<(), Error> {
    let dir = cli.dir;
    match cli.command {
        Command::Node { command } => node(command, &dir),
        Command::Fs { command } => fs(&Client::new(&dir)?, command),
        Command::Space { command } => space(&Client::new(&dir)?, command),
        Command::Identity { command } => identity(&Client::new(&dir)?, command, &dir),
        Command::Curl { args } => curl(&dir, args),
        Command::App {
            command: AppCommand::Pack { folder, file },
        } => {
            let data = fos_node::pack_app(&folder)?;
            // Writes only the file that was asked for, and never over another one.
            let mut out = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&file)
                .map_err(|e| Error(format!("{}: {e}", file.display())))?;
            out.write_all(&data)
                .map_err(|e| Error(format!("{}: {e}", file.display())))?;
            println!("packed {} ({} bytes)", file.display(), data.len());
            Ok(())
        }
        Command::App {
            command: AppCommand::Install { app },
        } => {
            let app = std::fs::canonicalize(&app)
                .map_err(|e| Error(format!("{}: {e}", app.display())))?;
            let client = Client::new(&dir)?;
            let answer = client.call(
                "POST",
                &format!("/v0/install?dir={}", enc(&app.to_string_lossy())),
                &[],
            )?;
            let answer = String::from_utf8_lossy(&answer);
            let mut lines = answer.lines();
            let (key, id) = (
                lines.next().unwrap_or_default(),
                lines.next().unwrap_or_default(),
            );
            let port = client.addr.rsplit(':').next().unwrap_or_default();
            println!("installed\napp key {key}\nopen    http://{id}.localhost:{port}/");
            Ok(())
        }
    }
}

fn node(command: NodeCommand, dir: &Path) -> Result<(), Error> {
    match command {
        NodeCommand::Init => {
            let key = Node::init(dir)?;
            println!("made a node in {}\nnode key {key}", dir.display());
            Ok(())
        }
        NodeCommand::Run {
            listen,
            peer,
            port,
            no_mdns,
        } => serve(dir, listen, peer, port, !no_mdns),
        NodeCommand::Status => print(&Client::new(dir)?.call("GET", "/v0/status", &[])?),
        NodeCommand::Connect { addr } => Client::new(dir)?
            .call("POST", "/v0/dial", addr.as_bytes())
            .map(drop),
    }
}

fn fs(client: &Client, command: FsCommand) -> Result<(), Error> {
    match command {
        FsCommand::Ls { path } => {
            print(&client.call("GET", &format!("/v0/fs{}?list", url_path(&path)), &[])?)
        }
        FsCommand::Cat { path } => {
            print(&client.call("GET", &format!("/v0/fs{}", url_path(&path)), &[])?)
        }
        FsCommand::Put { path, file } => {
            let data = if let Some(file) = file {
                std::fs::read(&file).map_err(|e| Error(format!("{}: {e}", file.display())))?
            } else {
                let mut data = Vec::new();
                std::io::stdin()
                    .read_to_end(&mut data)
                    .map_err(|e| Error(e.to_string()))?;
                data
            };
            client
                .call("PUT", &format!("/v0/fs{}", url_path(&path)), &data)
                .map(drop)
        }
        FsCommand::Append { path, text } => client
            .call(
                "POST",
                &format!("/v0/fs{}?append", url_path(&path)),
                format!("{text}\n").as_bytes(),
            )
            .map(drop),
        FsCommand::Rm { path } => client
            .call("DELETE", &format!("/v0/fs{}", url_path(&path)), &[])
            .map(drop),
    }
}

fn space(client: &Client, command: SpaceCommand) -> Result<(), Error> {
    match command {
        SpaceCommand::New { at } => {
            print(&client.call("POST", &format!("/v0/space?at={}", enc(&at)), &[])?)
        }
        SpaceCommand::Mount { id, at } => client
            .call(
                "POST",
                &format!("/v0/mount?id={}&at={}", enc(&id), enc(&at)),
                &[],
            )
            .map(drop),
        SpaceCommand::Grant {
            path,
            key,
            permissions,
        } => print(&client.call(
            "POST",
            &format!(
                "/v0/grants{}?to={}&permissions={}",
                url_path(&path),
                enc(&key),
                enc(&permissions)
            ),
            &[],
        )?),
        SpaceCommand::Grants { path } => {
            print(&client.call("GET", &format!("/v0/grants{}", url_path(&path)), &[])?)
        }
        SpaceCommand::Take { path, id } => client
            .call(
                "DELETE",
                &format!("/v0/grants{}?id={}", url_path(&path), enc(&id)),
                &[],
            )
            .map(drop),
        SpaceCommand::Checkpoint { path } => client
            .call("POST", &format!("/v0/checkpoint{}", url_path(&path)), &[])
            .map(drop),
    }
}

fn identity(client: &Client, command: Option<IdentityCommand>, dir: &Path) -> Result<(), Error> {
    let text = |data: Vec<u8>| String::from_utf8_lossy(&data).trim().to_string();
    match command {
        None => print(&client.call("GET", "/v0/id", &[])?),
        Some(IdentityCommand::New) => {
            let did = text(client.call("POST", "/v0/id/new", &[])?);
            println!(
                "identity {did}\nThis node holds the keys of the identity:\n  {}\n  {}\n\
                 Keep a copy of them in a safe place. With the rotation key you can \
                 change everything; without it, the identity cannot be changed.",
                dir.join("identity-rotation.key").display(),
                dir.join("identity-signing.key").display()
            );
            Ok(())
        }
        Some(IdentityCommand::AddNode { key }) => {
            let space =
                text(client.call("POST", &format!("/v0/id/node?key={}", enc(&key)), &[])?);
            println!("added node {key}\nOn that node, run: fos identity join {space}");
            Ok(())
        }
        Some(IdentityCommand::RemoveNode { key }) => {
            client.call("DELETE", &format!("/v0/id/node?key={}", enc(&key)), &[])?;
            println!(
                "took away node {key}\nIts certificate is deleted, and its grant on the \
                 identity space is taken back."
            );
            Ok(())
        }
        Some(IdentityCommand::Join { space }) => client
            .call("POST", &format!("/v0/id/join?space={}", enc(&space)), &[])
            .map(drop),
    }
}

/// Waits for Ctrl-C, or (on Unix) for SIGTERM, which service managers send.
async fn stopped() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        if let Ok(mut term) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

fn curl(dir: &Path, mut args: Vec<String>) -> Result<(), Error> {
    let config = dir.join("gateway.curl");
    let addr = std::fs::read_to_string(dir.join("gateway.addr"))
        .ok()
        .filter(|_| config.exists())
        .ok_or_else(|| {
            Error(format!(
                "no node is running for {}: start it with `fos node run`",
                dir.display()
            ))
        })?;
    let port = addr.trim().rsplit(':').next().unwrap_or_default();
    if let Some(last) = args.last_mut()
        && last.starts_with('/')
    {
        *last = format!("http://fos.localhost:{port}{last}");
    }
    let status = std::process::Command::new("curl")
        .arg("--config")
        .arg(&config)
        .args(&args)
        .status()
        .map_err(|e| Error(format!("fos curl needs curl: {e}")))?;
    std::process::exit(status.code().unwrap_or(1));
}

fn print(data: &[u8]) -> Result<(), Error> {
    std::io::stdout()
        .write_all(data)
        .map_err(|e| Error(e.to_string()))
}

fn serve(
    dir: &Path,
    listen: Vec<String>,
    peers: Vec<String>,
    port: u16,
    mdns: bool,
) -> Result<(), Error> {
    let parse = |list: Vec<String>| {
        list.iter()
            .map(|a| {
                a.parse()
                    .map_err(|e| Error(format!("{a} is not a multiaddr: {e}")))
            })
            .collect::<Result<Vec<_>, _>>()
    };
    let config = NetConfig {
        listen: parse(listen)?,
        peers: parse(peers)?,
        mdns,
    };
    let runtime = tokio::runtime::Runtime::new().map_err(|e| Error(e.to_string()))?;
    runtime.block_on(async {
        let node = Node::open(dir)?;
        let token = fos_node::token(dir)?;
        node.start_net(config)?;
        let port = fos_gateway::start(node.clone(), port, token).await?;
        fos_node::write_gateway_addr(dir, Some(port))?;
        eprintln!(
            "node {}\ngateway http://fos.localhost:{port}\n(stop with Ctrl-C)",
            node.key()
        );
        stopped().await;
        fos_node::write_gateway_addr(dir, None)
    })
}

/// A small HTTP/1.1 client for the gateway on this machine.
struct Client {
    addr: String,
    token: String,
}

impl Client {
    fn new(dir: &Path) -> Result<Self, Error> {
        let addr = std::fs::read_to_string(dir.join("gateway.addr")).map_err(|e| {
            Error(format!(
                "no node is running for {} ({e}): start it with `fos node run`",
                dir.display()
            ))
        })?;
        Ok(Self {
            addr: addr.trim().to_string(),
            token: fos_node::token(dir)?,
        })
    }

    fn call(&self, method: &str, target: &str, body: &[u8]) -> Result<Vec<u8>, Error> {
        let io = |e: std::io::Error| Error(format!("gateway {}: {e}", self.addr));
        let mut stream = TcpStream::connect(&self.addr).map_err(io)?;
        let head = format!(
            "{method} {target} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            self.addr,
            self.token,
            body.len()
        );
        stream
            .write_all(head.as_bytes())
            .and_then(|()| stream.write_all(body))
            .map_err(io)?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response).map_err(io)?;
        let split = response
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .ok_or_else(|| Error("a broken answer".into()))?;
        let head = String::from_utf8_lossy(&response[..split]);
        let body = response[split + 4..].to_vec();
        let status: u16 = head
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if status == 200 {
            Ok(body)
        } else {
            Err(Error(String::from_utf8_lossy(&body).trim().to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory as _;

    use super::*;

    #[test]
    fn the_commands_are_well_made() {
        Cli::command().debug_assert();
    }

    #[test]
    fn arguments_do_not_take_the_place_of_dir() {
        let cli =
            Cli::try_parse_from(["fos", "--dir", "/data", "app", "install", "Photos.app"]).unwrap();
        assert_eq!(cli.dir, PathBuf::from("/data"));
        let Command::App {
            command: AppCommand::Install { app },
        } = cli.command
        else {
            panic!("not app install");
        };
        assert_eq!(app, PathBuf::from("Photos.app"));
        let cli = Cli::try_parse_from(["fos", "id"]).unwrap();
        assert!(matches!(cli.command, Command::Identity { command: None }));
    }
}
