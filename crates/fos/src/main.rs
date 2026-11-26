// SPDX-License-Identifier: AGPL-3.0-only

//! `fos`: run a fileOS node, and use it from the command line.
//!
//! `fos init` and `fos run` work on the data directory. The other commands
//! talk to a running node through its gateway on this machine.

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
    /// Make a new node, with the engine in an .app directory.
    Init {
        #[arg(long)]
        engine: PathBuf,
    },
    /// Run the node: the network, the apps and the gateway.
    Run {
        /// libp2p addresses to listen on.
        #[arg(long, default_values = ["/ip4/0.0.0.0/tcp/0", "/ip6/::/tcp/0"])]
        listen: Vec<String>,
        /// Nodes to connect to at the start.
        #[arg(long)]
        peer: Vec<String>,
        /// The address of the gateway, on this machine only.
        #[arg(long, default_value = "127.0.0.1:0")]
        gateway: String,
        /// Do not look for nodes on the home network.
        #[arg(long)]
        no_mdns: bool,
    },
    /// What the node is doing.
    Status,
    /// Make a folder and mount it at a path.
    New { name: String, at: String },
    /// Mount a folder from other nodes at a path.
    Mount { id: String, at: String },
    /// The names under a path.
    Ls {
        #[arg(default_value = "/")]
        path: String,
    },
    /// Print a file or stream.
    Cat { path: String },
    /// Put a file. With no FILE, it reads standard input.
    Put { path: String, file: Option<PathBuf> },
    /// Add a line to the end of a stream.
    Append { path: String, text: String },
    /// Take a file or mount away.
    Rm { path: String },
    /// Give a key abilities (read, write, append, grant, store or all) on a path.
    Grant {
        path: String,
        key: String,
        abilities: String,
    },
    /// Write a policy entry for the engine into the folder that has a path.
    Policy { path: String, text: String },
    /// Install an app from an .app directory.
    Install { app: PathBuf },
    /// Show the local key-value store of an app.
    Kv { app: String },
    /// Connect to another node by its multiaddr.
    Dial { addr: String },
    /// The identity of this node. With no command, show it.
    Id {
        #[command(subcommand)]
        command: Option<IdCommand>,
    },
}

#[derive(Subcommand)]
enum IdCommand {
    /// Make a new identity (a did:fos DID). This node holds its keys and is its first node.
    New,
    /// Add a node to the identity. Only the node that holds the signing key can.
    AddNode { key: String },
    /// Take a node away from the identity.
    RemoveNode { key: String },
    /// Join an identity, with the folder ID that `fos id add-node` gave.
    Join { folder: String },
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
    let client = || Client::new(&dir);
    match cli.command {
        Command::Init { engine } => {
            let key = Node::init(&dir, &engine)?;
            println!("made a node in {}\nnode key {key}", dir.display());
            Ok(())
        }
        Command::Run {
            listen,
            peer,
            gateway,
            no_mdns,
        } => serve(&dir, listen, peer, &gateway, !no_mdns),
        Command::Status => print(&client()?.call("GET", "/v0/status", &[])?),
        Command::New { name, at } => print(&client()?.call(
            "POST",
            &format!("/v0/folder?name={}&at={}", enc(&name), enc(&at)),
            &[],
        )?),
        Command::Mount { id, at } => client()?
            .call(
                "POST",
                &format!("/v0/mount?id={}&at={}", enc(&id), enc(&at)),
                &[],
            )
            .map(drop),
        Command::Ls { path } => {
            print(&client()?.call("GET", &format!("/v0/fs{}?list", url_path(&path)), &[])?)
        }
        Command::Cat { path } => {
            let data = client()?.call("GET", &format!("/v0/fs{}", url_path(&path)), &[])?;
            std::io::stdout()
                .write_all(&data)
                .map_err(|e| Error(e.to_string()))
        }
        Command::Put { path, file } => {
            let data = if let Some(file) = file {
                std::fs::read(&file).map_err(|e| Error(format!("{}: {e}", file.display())))?
            } else {
                let mut data = Vec::new();
                std::io::stdin()
                    .read_to_end(&mut data)
                    .map_err(|e| Error(e.to_string()))?;
                data
            };
            client()?
                .call("PUT", &format!("/v0/fs{}", url_path(&path)), &data)
                .map(drop)
        }
        Command::Append { path, text } => client()?
            .call(
                "POST",
                &format!("/v0/fs{}?append", url_path(&path)),
                format!("{text}\n").as_bytes(),
            )
            .map(drop),
        Command::Rm { path } => client()?
            .call("DELETE", &format!("/v0/fs{}", url_path(&path)), &[])
            .map(drop),
        Command::Grant {
            path,
            key,
            abilities,
        } => client()?
            .call(
                "POST",
                &format!(
                    "/v0/grant{}?to={}&abilities={}",
                    url_path(&path),
                    enc(&key),
                    enc(&abilities)
                ),
                &[],
            )
            .map(drop),
        Command::Policy { path, text } => client()?
            .call(
                "POST",
                &format!("/v0/policy{}", url_path(&path)),
                text.as_bytes(),
            )
            .map(drop),
        Command::Install { app } => {
            let app = std::fs::canonicalize(&app)
                .map_err(|e| Error(format!("{}: {e}", app.display())))?;
            let key = client()?.call(
                "POST",
                &format!("/v0/install?dir={}", enc(&app.to_string_lossy())),
                &[],
            )?;
            print(&[b"installed; app key ", key.as_slice()].concat())
        }
        Command::Kv { app } => {
            print(&client()?.call("GET", &format!("/v0/kv/{}", enc(&app)), &[])?)
        }
        Command::Dial { addr } => client()?
            .call("POST", "/v0/dial", addr.as_bytes())
            .map(drop),
        Command::Id { command } => id(&client()?, command, &dir),
    }
}

fn id(client: &Client, command: Option<IdCommand>, dir: &Path) -> Result<(), Error> {
    let text = |data: Vec<u8>| String::from_utf8_lossy(&data).trim().to_string();
    match command {
        None => print(&client.call("GET", "/v0/id", &[])?),
        Some(IdCommand::New) => {
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
        Some(IdCommand::AddNode { key }) => {
            let folder =
                text(client.call("POST", &format!("/v0/id/node?key={}", enc(&key)), &[])?);
            println!("added node {key}\nOn that node, run: fos id join {folder}");
            Ok(())
        }
        Some(IdCommand::RemoveNode { key }) => {
            client.call("DELETE", &format!("/v0/id/node?key={}", enc(&key)), &[])?;
            println!(
                "took away node {key}\nIt may still keep its copy of the identity folder \
                 until your engine takes its grant away."
            );
            Ok(())
        }
        Some(IdCommand::Join { folder }) => client
            .call("POST", &format!("/v0/id/join?folder={}", enc(&folder)), &[])
            .map(drop),
    }
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
    gateway: &str,
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
    let gateway = gateway
        .parse()
        .map_err(|e| Error(format!("{gateway}: {e}")))?;
    let runtime = tokio::runtime::Runtime::new().map_err(|e| Error(e.to_string()))?;
    runtime.block_on(async {
        let node = Node::open(dir)?;
        let token = fos_node::token(dir)?;
        node.start_net(config)?;
        let addr = fos_gateway::start(node.clone(), gateway, token).await?;
        fos_node::write_gateway_addr(dir, Some(addr))?;
        eprintln!(
            "node {}\ngateway http://{addr}\n(stop with Ctrl-C)",
            node.key()
        );
        let _ = tokio::signal::ctrl_c().await;
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
                "no node is running for {} ({e}): start it with `fos run`",
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
