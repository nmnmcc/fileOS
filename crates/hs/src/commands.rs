// SPDX-License-Identifier: AGPL-3.0-only

//! Commands that work on a node's content: on the data directory when the
//! node is stopped, else through the API of a running node.

use std::collections::BTreeMap;
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{Args, Subcommand};
use clap_complete::engine::ArgValueCandidates;
use hs_api::client::{Reply, resource_path};
use hs_api::replica;
use hs_api::types as t;
use hs_api::{Api, types::Role};
use hs_node::Node;
use http::{Method, StatusCode, header};
use serde_json::{Value as Json, json};

use crate::output::{print_json, short, size, table, time};
use crate::remote::{self, Server};
use crate::{Res, RoleArg};

/// The options every command takes.
pub struct Global {
    /// `--dir` or `$HS_DIR`.
    pub dir: Option<PathBuf>,
    /// `--server` or `$HS_SERVER`.
    pub server: Option<String>,
    pub json: bool,
}

/// Where a command does its work.
pub enum Target {
    /// The data directory, opened (so no server is using it).
    Local(Arc<Node>),
    Remote(Server),
}

impl Global {
    /// `--server` means that server and `--dir` that directory. Otherwise
    /// the default data directory, unless a running node holds it (or there
    /// is none) and a sign-in is saved.
    pub async fn target(&self) -> Res<Target> {
        if self.server.is_some() {
            return Ok(Target::Remote(remote::required(self.server.as_deref())?));
        }
        if let Some(dir) = &self.dir {
            return Ok(Target::Local(Arc::new(crate::open(dir).await?)));
        }
        let dir = crate::data_dir(None);
        let saved = remote::configured(None)?;
        if dir.join("config.json").exists() || saved.is_none() {
            match crate::open(&dir).await {
                Ok(node) => return Ok(Target::Local(Arc::new(node))),
                Err(e) if saved.is_none() => return Err(e),
                Err(_) => {}
            }
        }
        Ok(Target::Remote(saved.expect("checked above")))
    }

    /// The signed-in server, for what only a running node does.
    pub fn server(&self) -> Res<Server> {
        remote::required(self.server.as_deref())
    }
}

impl Target {
    pub(crate) async fn close(self) {
        if let Target::Local(node) = self {
            node.index.close().await;
        }
    }
}

// Spaces

#[derive(Subcommand)]
pub enum SpaceCommand {
    /// List spaces
    #[command(visible_alias = "ls")]
    List,
    /// Show a space and its members
    Show {
        #[arg(add = ArgValueCandidates::new(crate::complete::spaces))]
        space: String,
    },
    /// Create a space
    Create { name: String },
    /// Join a space kept on another node (its owner adds this node first)
    Join {
        /// The URL of the node that keeps the space
        #[arg(long, value_name = "URL")]
        home: String,
        /// The space ID
        space: String,
    },
    /// Add or remove member nodes
    #[command(subcommand)]
    Member(MemberCommand),
    /// Make another member node the one that keeps the space
    Handover {
        #[arg(add = ArgValueCandidates::new(crate::complete::spaces))]
        space: String,
        node: String,
    },
    /// Pull changes from the node that keeps the space, now
    Sync {
        #[arg(add = ArgValueCandidates::new(crate::complete::spaces))]
        space: String,
    },
}

#[derive(Subcommand)]
pub enum MemberCommand {
    /// Add a node, by its URL (it is asked who it is) or by its keys
    Add {
        #[arg(add = ArgValueCandidates::new(crate::complete::spaces))]
        space: String,
        /// The node's URL
        #[arg(long, required_unless_present = "node")]
        url: Option<String>,
        /// The node's key (`did:…`)
        #[arg(long, requires = "identity", conflicts_with = "url")]
        node: Option<String>,
        /// Its owner's identity (`did:…`)
        #[arg(long)]
        identity: Option<String>,
        #[arg(long, value_enum, default_value_t = RoleArg::Writer)]
        role: RoleArg,
    },
    /// Remove a member node
    Remove {
        #[arg(add = ArgValueCandidates::new(crate::complete::spaces))]
        space: String,
        node: String,
    },
}

/// Space IDs (hex) to names.
pub(crate) async fn space_names(t: &Target) -> BTreeMap<String, String> {
    let mut names = BTreeMap::new();
    match t {
        Target::Local(node) => {
            for id in node.space_ids() {
                if let Ok(name) = node.with_space(&id, |l| l.space.name().to_string()).await {
                    names.insert(id.to_hex(), name);
                }
            }
        }
        Target::Remote(s) => return remote_names(s).await,
    }
    names
}

async fn remote_names(s: &Server) -> BTreeMap<String, String> {
    s.spaces()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|sp| (sp.id, sp.name))
        .collect()
}

fn name_of(names: &BTreeMap<String, String>, id: &str) -> String {
    names.get(id).cloned().unwrap_or_else(|| short(id, 12))
}

pub async fn space(g: &Global, cmd: SpaceCommand) -> Res {
    match cmd {
        SpaceCommand::List => {
            let t = g.target().await?;
            let list = match &t {
                Target::Local(node) => {
                    let api = Api::new(Arc::clone(node));
                    let mut out = Vec::new();
                    for id in node.space_ids() {
                        out.push(api.space_json(&id).await?);
                    }
                    out
                }
                Target::Remote(s) => s.spaces().await?,
            };
            t.close().await;
            if g.json {
                print_json(&list);
                return Ok(());
            }
            let rows: Vec<Vec<String>> = list
                .iter()
                .map(|sp| {
                    vec![
                        short(&sp.id, 12),
                        sp.name.clone(),
                        sp.members.len().to_string(),
                        if sp.is_primary {
                            "this node".into()
                        } else {
                            sp.home.clone().unwrap_or_default()
                        },
                    ]
                })
                .collect();
            table(&["ID", "NAME", "MEMBERS", "KEPT BY"], &rows);
            Ok(())
        }
        SpaceCommand::Create { name } => {
            let t = g.target().await?;
            let made = match &t {
                Target::Local(node) => {
                    let api = Api::new(Arc::clone(node));
                    let id = api.create_space(&name, json!({})).await?;
                    api.space_json(&id).await?
                }
                Target::Remote(s) => s.create_space(&t::NewSpace { name, info: None }).await?,
            };
            t.close().await;
            if g.json {
                print_json(&made);
            } else {
                println!("{}", made.id);
            }
            Ok(())
        }
        SpaceCommand::Show { space } => {
            let s = g.server()?;
            let sp = s.space(&space).await?;
            show_space(g, &sp);
            Ok(())
        }
        SpaceCommand::Join { home, space } => {
            let s = g.server()?;
            let home = remote::normalize_url(&home)?;
            let sp = s.join(&t::Join { home, space }).await?;
            show_space(g, &sp);
            Ok(())
        }
        SpaceCommand::Member(MemberCommand::Add {
            space,
            url,
            node,
            identity,
            role,
        }) => {
            let s = g.server()?;
            let new = t::NewMember {
                node,
                identity,
                role: Some(role.into()),
                url: url.as_deref().map(remote::normalize_url).transpose()?,
                home: None,
            };
            let r = s.add_member(&space, &new).await?;
            done(g, &r, "added");
            Ok(())
        }
        SpaceCommand::Member(MemberCommand::Remove { space, node }) => {
            let s = g.server()?;
            s.remove_member(&space, &node).await?;
            done(g, &Json::Null, "removed");
            Ok(())
        }
        SpaceCommand::Handover { space, node } => {
            let s = g.server()?;
            let r = s.handover(&space, &node).await?;
            done(g, &r, "handed over");
            Ok(())
        }
        SpaceCommand::Sync { space } => {
            let s = g.server()?;
            let r = s.pull(&space).await?;
            if g.json {
                print_json(&r);
            } else {
                println!("pulled {} changes", r.pulled);
            }
            Ok(())
        }
    }
}

/// Say it worked, or print the reply with `--json`.
fn done(g: &Global, reply: &impl serde::Serialize, what: &str) {
    if g.json {
        print_json(reply);
    } else {
        println!("{what}");
    }
}

fn show_space(g: &Global, sp: &t::Space) {
    if g.json {
        print_json(sp);
        return;
    }
    println!("ID       {}", sp.id);
    println!("Name     {}", sp.name);
    println!("Changes  {}", sp.seq);
    if sp.is_primary {
        println!("Kept by  this node");
    } else {
        println!(
            "Kept by  {} ({})",
            sp.primary,
            sp.home.as_deref().unwrap_or("")
        );
    }
    println!();
    let rows: Vec<Vec<String>> = sp
        .members
        .iter()
        .map(|m| {
            vec![
                m.role.as_str().to_string(),
                m.node.clone(),
                m.identity.clone(),
                m.home.clone().unwrap_or_default(),
            ]
        })
        .collect();
    table(&["ROLE", "NODE", "IDENTITY", "URL"], &rows);
}

// Tokens

#[derive(Subcommand)]
pub enum TokenCommand {
    /// Create a token; prints it once
    Create {
        #[arg(long, value_enum, default_value_t = RoleArg::Writer)]
        role: RoleArg,
        /// What the token is for
        #[arg(long, default_value = "created by hs ctl token create")]
        label: String,
        /// Limit the token to these spaces (name or ID) [default: all]
        #[arg(long = "space", value_name = "SPACE", add = ArgValueCandidates::new(crate::complete::spaces))]
        spaces: Vec<String>,
        /// Limit the token to these resource types [default: all]
        #[arg(long = "type", value_name = "TYPE")]
        types: Vec<String>,
        /// Make it stop working after this long: 90m, 12h, 30d, 4w
        #[arg(long, value_name = "DURATION", value_parser = parse_duration)]
        expires_in: Option<u64>,
    },
    /// List grants
    #[command(visible_alias = "ls")]
    List,
    /// Revoke a grant
    Revoke {
        /// The grant ID, from `hs ctl token list`
        #[arg(add = ArgValueCandidates::new(crate::complete::grants))]
        id: String,
    },
}

/// The grants of a node.
pub(crate) async fn grants(t: &Target) -> Res<Vec<t::Grant>> {
    Ok(match t {
        Target::Local(node) => node
            .grants()
            .await
            .into_iter()
            .filter_map(|g| serde_json::from_value(g).ok())
            .collect(),
        Target::Remote(s) => s.grants().await?,
    })
}

/// `90m`, `12h`, `30d` and the like, in milliseconds.
fn parse_duration(s: &str) -> Result<u64, String> {
    let split = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (n, unit) = s.split_at(split);
    let n: u64 = n.parse().map_err(|_| format!("not a duration: {s}"))?;
    let unit: u64 = match unit {
        "s" => 1_000,
        "m" | "min" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        "w" => 604_800_000,
        _ => return Err(format!("not a duration: {s} (use s, m, h, d or w)")),
    };
    n.checked_mul(unit).ok_or_else(|| "too long".into())
}

pub async fn token(g: &Global, cmd: TokenCommand) -> Res {
    let t = g.target().await?;
    let out = token_on(g, &t, cmd).await;
    t.close().await;
    out
}

async fn token_on(g: &Global, t: &Target, cmd: TokenCommand) -> Res {
    match cmd {
        TokenCommand::Create {
            role,
            label,
            spaces,
            types,
            expires_in,
        } => {
            let new = t::NewGrant {
                spaces: (!spaces.is_empty()).then_some(spaces),
                types: (!types.is_empty()).then_some(types),
                expires: expires_in.map(|ms| hs_node::now_ms().saturating_add(ms)),
                ..t::NewGrant::new(role.into(), &label)
            };
            let made = match t {
                Target::Local(node) => Api::new(Arc::clone(node)).create_grant(&new).await?,
                Target::Remote(s) => s.create_grant(&new).await?,
            };
            if g.json {
                print_json(&made);
            } else {
                eprintln!("grant {}", made.id);
                println!("{}", made.token);
            }
        }
        TokenCommand::List => {
            let list = grants(t).await?;
            if g.json {
                print_json(&list);
                return Ok(());
            }
            let names = space_names(t).await;
            let all = |v: &Option<Vec<String>>, f: &dyn Fn(&str) -> String| match v {
                Some(a) => a.iter().map(|s| f(s)).collect::<Vec<_>>().join(","),
                None => "all".into(),
            };
            let rows: Vec<Vec<String>> = list
                .iter()
                .map(|gr| {
                    vec![
                        gr.id.clone(),
                        gr.role.as_str().to_string(),
                        all(&gr.spaces, &|s| name_of(&names, s)),
                        all(&gr.types, &str::to_string),
                        time(gr.created),
                        time(gr.expires.unwrap_or(0)),
                        gr.label.clone(),
                    ]
                })
                .collect();
            table(
                &[
                    "ID", "ROLE", "SPACES", "TYPES", "CREATED", "EXPIRES", "LABEL",
                ],
                &rows,
            );
        }
        TokenCommand::Revoke { id } => {
            match t {
                Target::Local(node) => node.revoke(&id).await.map_err(|e| e.to_string())?,
                Target::Remote(s) => s.revoke_grant(&id).await?,
            }
            eprintln!("revoked {id}");
        }
    }
    Ok(())
}

// Identity, export, replicas

pub async fn identity(g: &Global) -> Res {
    let t = g.target().await?;
    let doc = match &t {
        Target::Local(node) => hs_identity::did::document(&node.identity()),
        Target::Remote(s) => s.identity().await?,
    };
    t.close().await;
    print_json(&doc);
    Ok(())
}

pub async fn export(g: &Global) -> Res {
    let out = std::io::stdout();
    if out.is_terminal() {
        return Err("redirect the archive to a file: hs ctl export > backup.tar".into());
    }
    match g.target().await? {
        Target::Local(node) => {
            let dir = node.dir().to_path_buf();
            // The archive reads the files; the node only holds the lock.
            let mut out = std::io::BufWriter::new(out.lock());
            let r = hs_api::export::archive(&dir, &mut out)
                .map_err(|e| e.to_string())
                .and_then(|()| out.flush().map_err(|e| e.to_string()));
            drop(out);
            node.index.close().await;
            r
        }
        Target::Remote(s) => {
            let mut out = std::io::BufWriter::new(out.lock());
            s.download("/api/export", &mut out).await?;
            Ok(())
        }
    }
}

#[derive(Subcommand)]
pub enum ReplicaCommand {
    /// List configured replicas
    #[command(visible_alias = "ls")]
    List,
    /// Bring every replica up to date now
    Sync,
}

pub async fn replica(g: &Global, cmd: ReplicaCommand) -> Res {
    let t = g.target().await?;
    let list = match (&t, &cmd) {
        (Target::Local(node), ReplicaCommand::List) => serde_json::to_value(
            replica::configured(&Api::new(Arc::clone(node)))
                .iter()
                .map(|s| replica::redact(s))
                .collect::<Vec<_>>(),
        ),
        (Target::Local(node), ReplicaCommand::Sync) => {
            serde_json::to_value(replica::sync_all(&Api::new(Arc::clone(node))).await)
        }
        (Target::Remote(s), ReplicaCommand::List) => serde_json::to_value(s.replicas().await?),
        (Target::Remote(s), ReplicaCommand::Sync) => serde_json::to_value(s.sync_replicas().await?),
    }
    .map_err(|e| e.to_string())?;
    t.close().await;
    let items = list.as_array().cloned().unwrap_or_default();
    if g.json {
        print_json(&list);
    } else if items.is_empty() {
        eprintln!("no replicas; add one with hs ctl config set replicas '[\"s3://…\"]'");
    } else {
        for item in items {
            match item {
                Json::String(s) => println!("{s}"),
                other => println!("{other}"),
            }
        }
    }
    Ok(())
}

// Signing in

pub async fn login(url: Option<String>, insecure: bool) -> Res {
    let url = remote::normalize_url(url.as_deref().unwrap_or(remote::LOCAL_URL))?;
    let token = match std::env::var("HS_TOKEN") {
        Ok(t) if !t.is_empty() => t,
        _ => remote::read_token()?,
    };
    let s = Server::new(url.clone(), token.clone(), insecure);
    let session = s.session().await?;
    let Some(role) = session.role else {
        return Err(format!("{url} did not accept that token"));
    };
    let node = s.node().await?;
    remote::save_login(&url, &token, insecure)?;
    println!("Signed in to {} ({url}) as {}", node.name, role.as_str());
    Ok(())
}

pub fn logout(url: Option<String>) -> Res {
    match remote::forget_login(url.as_deref())? {
        Some(url) => {
            println!("Signed out of {url}");
            Ok(())
        }
        None => Err("not signed in there".into()),
    }
}

pub async fn status(g: &Global) -> Res {
    let s = g.server()?;
    let node = s.node().await?;
    let session = s.session().await?;
    let health = s.health().await?;
    if g.json {
        print_json(&json!({ "node": node, "session": session, "health": health }));
        return Ok(());
    }
    let resources: u64 = health.counts.values().sum();
    println!("Server     {}", s.url);
    println!("Node       {}", node.name);
    println!("Identity   {}", node.did);
    println!("Version    {}", node.version);
    println!(
        "Signed in  {}{}",
        session.role.map_or("", Role::as_str),
        match &session.grant {
            Some(id) => format!(" (grant {id})"),
            None => String::new(),
        }
    );
    println!(
        "Index      {}",
        if health.index {
            "healthy"
        } else {
            "needs a rebuild (hs ctl rebuild-index)"
        }
    );
    println!("Content    {} spaces, {resources} resources", health.spaces);
    let others: Vec<String> = remote::logins()?
        .into_iter()
        .filter(|(u, _)| *u != s.url)
        .map(|(u, _)| u)
        .collect();
    if !others.is_empty() {
        println!("Also       {}", others.join(", "));
    }
    Ok(())
}

// Resources

#[derive(Args)]
pub struct LsArgs {
    /// The space (name or ID)
    #[arg(add = ArgValueCandidates::new(crate::complete::spaces))]
    space: String,
    /// Only this type
    #[arg(value_name = "TYPE")]
    kind: Option<String>,
    /// How many to list
    #[arg(long, default_value_t = 50)]
    limit: u32,
    /// Skip this many
    #[arg(long, default_value_t = 0)]
    offset: u32,
    /// List them all, page by page
    #[arg(long, conflicts_with_all = ["limit", "offset"])]
    all: bool,
}

pub async fn ls(g: &Global, a: LsArgs) -> Res {
    let s = g.server()?;
    let mut q = t::ListQuery {
        kind: a.kind,
        limit: Some(if a.all { 1000 } else { a.limit }),
        offset: Some(a.offset),
        page: None,
    };
    let mut items = Vec::new();
    let more = loop {
        let page = s.resources(&a.space, &q).await?;
        items.extend(page.items);
        match page.next_offset {
            Some(next) if a.all => q.offset = Some(next),
            next => break next,
        }
    };
    if g.json {
        print_json(&items);
        return Ok(());
    }
    let rows: Vec<Vec<String>> = items
        .iter()
        .map(|r| vec![r.target.clone(), time(r.updated), r.title.clone()])
        .collect();
    table(&["RESOURCE", "UPDATED", "TITLE"], &rows);
    if let Some(next) = more {
        eprintln!("(more: --offset {next}, or --all)");
    }
    Ok(())
}

#[derive(Args)]
pub struct GetArgs {
    /// The space (name or ID)
    #[arg(add = ArgValueCandidates::new(crate::complete::spaces))]
    space: String,
    /// TYPE/ID, such as note/shopping
    target: String,
    /// Write it to this file
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,
}

pub async fn get(g: &Global, a: GetArgs) -> Res {
    let s = g.server()?;
    let r = s
        .send(Method::GET, &resource_path(&a.space, &a.target)?, &[], None)
        .await?;
    if let Some(path) = &a.output {
        return std::fs::write(path, &r.body).map_err(|e| format!("{}: {e}", path.display()));
    }
    let is_json = r
        .header(header::CONTENT_TYPE)
        .is_some_and(|t| t.starts_with("application/json"));
    let mut out = std::io::stdout().lock();
    if is_json && out.is_terminal() {
        print_json(&r.json()?);
        return Ok(());
    }
    out.write_all(&r.body)
        .and_then(|()| out.flush())
        .map_err(|e| e.to_string())
}

/// The media type for a file's name, or for JSON content.
fn guess_media_type(path: Option<&Path>, bytes: &[u8]) -> Option<&'static str> {
    let ext = path
        .and_then(Path::extension)
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let by_name = match ext.as_deref() {
        Some("json") => Some("application/json"),
        Some("vcf" | "vcard") => Some("text/vcard"),
        Some("ics") => Some("text/calendar"),
        Some("txt") => Some("text/plain; charset=utf-8"),
        Some("md") => Some("text/markdown; charset=utf-8"),
        Some("html" | "htm") => Some("text/html; charset=utf-8"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("png") => Some("image/png"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        Some("heic") => Some("image/heic"),
        Some("svg") => Some("image/svg+xml"),
        Some("pdf") => Some("application/pdf"),
        Some("mp4") => Some("video/mp4"),
        Some("mov") => Some("video/quicktime"),
        Some("mp3") => Some("audio/mpeg"),
        Some("m4a") => Some("audio/mp4"),
        _ => None,
    };
    by_name.or_else(|| {
        serde_json::from_slice::<Json>(bytes)
            .is_ok()
            .then_some("application/json")
    })
}

/// The content of FILE, or of stdin for `-` or none.
fn read_input(file: Option<&Path>) -> Res<Vec<u8>> {
    match file {
        Some(p) if p != Path::new("-") => {
            std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))
        }
        _ => {
            let stdin = std::io::stdin();
            if stdin.is_terminal() {
                return Err("give a FILE, or pipe the content in".into());
            }
            let mut b = Vec::new();
            stdin
                .lock()
                .read_to_end(&mut b)
                .map_err(|e| e.to_string())?;
            Ok(b)
        }
    }
}

#[derive(Args)]
pub struct PutArgs {
    /// The space (name or ID)
    #[arg(add = ArgValueCandidates::new(crate::complete::spaces))]
    space: String,
    /// TYPE/ID, such as note/shopping
    target: String,
    /// The content; `-` or nothing reads stdin
    file: Option<PathBuf>,
    /// The media type [default: from the file name, or JSON]
    #[arg(long, value_name = "TYPE")]
    media_type: Option<String>,
    /// Fail if it already exists
    #[arg(long, conflicts_with = "if_match")]
    new: bool,
    /// Fail unless it is still this version (an `ETag`)
    #[arg(long, value_name = "ETAG")]
    if_match: Option<String>,
}

pub async fn put(g: &Global, a: PutArgs) -> Res {
    let s = g.server()?;
    let path = resource_path(&a.space, &a.target)?;
    let bytes = read_input(a.file.as_deref())?;
    let media_type = match a.media_type.as_deref() {
        Some(t) => t,
        None => guess_media_type(a.file.as_deref(), &bytes)
            .ok_or("what kind of content is it? Say with --media-type")?,
    };
    let mut headers = Vec::new();
    if a.new {
        headers.push((header::IF_NONE_MATCH, "*".to_string()));
    }
    if let Some(tag) = a.if_match {
        headers.push((header::IF_MATCH, quoted(&tag)));
    }
    let r = s
        .send(Method::PUT, &path, &headers, Some((media_type, bytes)))
        .await?;
    written(g, &a.target, &r)
}

/// An `ETag` as given, or in quotes if it came without.
fn quoted(tag: &str) -> String {
    if tag.starts_with('"') || tag.starts_with("W/") || tag == "*" {
        tag.to_string()
    } else {
        format!("\"{tag}\"")
    }
}

fn written(g: &Global, target: &str, r: &Reply) -> Res {
    let w: t::Written = r.typed()?;
    if g.json {
        print_json(&w);
    } else {
        let what = if r.status == StatusCode::CREATED {
            "created"
        } else {
            "updated"
        };
        println!("{what} {target} (change {})", w.seq);
    }
    Ok(())
}

#[derive(Args)]
pub struct EditArgs {
    /// The space (name or ID)
    #[arg(add = ArgValueCandidates::new(crate::complete::spaces))]
    space: String,
    /// TYPE/ID; a new one starts empty
    target: String,
}

pub async fn edit(g: &Global, a: EditArgs) -> Res {
    let s = g.server()?;
    let path = resource_path(&a.space, &a.target)?;
    let r = s.fetch(Method::GET, &path, &[], None).await?;
    let (before, media_type, condition) = if r.status == StatusCode::NOT_FOUND {
        (
            b"{\n}\n".to_vec(),
            "application/json".to_string(),
            (header::IF_NONE_MATCH, "*".to_string()),
        )
    } else {
        s.check(&r)?;
        let media_type = r
            .header(header::CONTENT_TYPE)
            .unwrap_or("application/octet-stream")
            .to_string();
        let tag = r
            .header(header::ETAG)
            .ok_or("the server sent no ETag")?
            .to_string();
        let body = if media_type.starts_with("application/json") {
            let v = r.json()?;
            let mut b = serde_json::to_vec_pretty(&v).unwrap_or_default();
            b.push(b'\n');
            b
        } else if media_type.starts_with("text/") {
            r.body
        } else {
            return Err(format!("{} is {media_type}, not text", a.target));
        };
        (body, media_type, (header::IF_MATCH, tag))
    };
    let is_json = media_type.starts_with("application/json");
    let mut file = tempfile::Builder::new()
        .prefix("hs-edit-")
        .suffix(if is_json { ".json" } else { ".txt" })
        .tempfile()
        .map_err(|e| e.to_string())?;
    file.write_all(&before)
        .and_then(|()| file.flush())
        .map_err(|e| e.to_string())?;
    run_editor(file.path())?;
    let after = std::fs::read(file.path()).map_err(|e| e.to_string())?;
    if after == before {
        eprintln!("no change");
        return Ok(());
    }
    let keep = |file: tempfile::NamedTempFile, why: String| -> String {
        match file.keep() {
            Ok((_, p)) => format!("{why}\nyour edit is kept in {}", p.display()),
            Err(_) => why,
        }
    };
    if is_json && let Err(e) = serde_json::from_slice::<Json>(&after) {
        return Err(keep(file, format!("that is not valid JSON: {e}")));
    }
    let media_type = media_type.split(';').next().unwrap_or("").trim();
    match s
        .send(Method::PUT, &path, &[condition], Some((media_type, after)))
        .await
    {
        Ok(r) => written(g, &a.target, &r),
        Err(e) => Err(keep(file, e.into())),
    }
}

/// Open `$VISUAL` or `$EDITOR` (else `vi`) on a file and wait for it.
fn run_editor(path: &Path) -> Res {
    let editor = ["VISUAL", "EDITOR"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|e| !e.trim().is_empty())
        .unwrap_or_else(|| "vi".into());
    // Through the shell, so `EDITOR="code --wait"` works.
    #[cfg(unix)]
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg("sh")
        .arg(path)
        .status();
    #[cfg(not(unix))]
    let status = std::process::Command::new(&editor).arg(path).status();
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("{editor} failed ({s})")),
        Err(e) => Err(format!("cannot run {editor}: {e}")),
    }
}

#[derive(Args)]
pub struct PatchArgs {
    /// The space (name or ID)
    #[arg(add = ArgValueCandidates::new(crate::complete::spaces))]
    space: String,
    /// TYPE/ID of a JSON resource
    target: String,
    /// A JSON merge patch (RFC 7396), such as '{"done": true}'; `-` reads stdin
    patch: String,
}

pub async fn patch(g: &Global, a: PatchArgs) -> Res {
    let s = g.server()?;
    let path = resource_path(&a.space, &a.target)?;
    let bytes = if a.patch == "-" {
        read_input(None)?
    } else {
        a.patch.into_bytes()
    };
    serde_json::from_slice::<Json>(&bytes).map_err(|e| format!("the patch is not JSON: {e}"))?;
    let r = s
        .send(
            Method::PATCH,
            &path,
            &[],
            Some(("application/merge-patch+json", bytes)),
        )
        .await?;
    written(g, &a.target, &r)
}

#[derive(Args)]
pub struct RmArgs {
    /// The space (name or ID)
    #[arg(add = ArgValueCandidates::new(crate::complete::spaces))]
    space: String,
    /// TYPE/ID
    target: String,
    /// Also remove its past versions from every member's log
    #[arg(long)]
    erase: bool,
}

pub async fn rm(g: &Global, a: RmArgs) -> Res {
    let s = g.server()?;
    if a.erase {
        s.erase(&a.space, &a.target).await?;
        eprintln!("erased {}", a.target);
    } else {
        s.delete_resource(&a.space, &a.target).await?;
        eprintln!("deleted {}", a.target);
    }
    Ok(())
}

#[derive(Args)]
pub struct UploadArgs {
    /// The space (name or ID)
    #[arg(add = ArgValueCandidates::new(crate::complete::spaces))]
    space: String,
    /// Files to upload
    #[arg(required = true)]
    files: Vec<PathBuf>,
}

/// Each file goes up as a blob, then a `file/…` resource names it. The ID
/// comes from the content, so uploading the same file again changes
/// nothing.
pub async fn upload(g: &Global, a: UploadArgs) -> Res {
    let s = g.server()?;
    let mut made = Vec::new();
    for path in &a.files {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let media_type = guess_media_type(Some(path), &[]).unwrap_or("application/octet-stream");
        let len = bytes.len();
        let hash = s.put_blob(media_type, bytes).await?.hash;
        let target = format!("file/{}", short(&hash, 16));
        let file = json!({
            "name": name,
            "hash": hash,
            "size": len,
            "media_type": media_type,
        });
        let r = s
            .send(
                Method::PUT,
                &resource_path(&a.space, &target)?,
                &[],
                Some((
                    "application/json",
                    serde_json::to_vec(&file).unwrap_or_default(),
                )),
            )
            .await?;
        if g.json {
            let w: t::Written = r.typed()?;
            made.push(json!({ "target": target, "hash": hash, "reply": w }));
        } else {
            println!("{target}  {}  {name}", size(len as u64));
        }
    }
    if g.json {
        print_json(&Json::Array(made));
    }
    Ok(())
}

// Finding things

/// Rows from search, the timeline and the like.
async fn print_rows(g: &Global, s: &Server, rows: &[t::Row]) -> Res {
    if g.json {
        print_json(rows);
        return Ok(());
    }
    let names = remote_names(s).await;
    let out: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                name_of(&names, &r.space),
                r.target.clone(),
                time(r.time),
                r.title.clone(),
            ]
        })
        .collect();
    table(&["SPACE", "RESOURCE", "TIME", "TITLE"], &out);
    Ok(())
}

#[derive(Args)]
pub struct SearchArgs {
    /// Words to look for
    #[arg(required = true)]
    words: Vec<String>,
    #[arg(long, default_value_t = 50)]
    limit: u32,
}

pub async fn search(g: &Global, a: SearchArgs) -> Res {
    let s = g.server()?;
    let rows = s
        .search(&t::SearchQuery {
            q: a.words.join(" "),
            limit: Some(a.limit),
        })
        .await?;
    print_rows(g, &s, &rows).await
}

#[derive(Args)]
pub struct TimelineArgs {
    /// Only what is from before this day (YYYY-MM-DD, UTC)
    #[arg(long, value_name = "DATE", value_parser = parse_date)]
    before: Option<u64>,
    #[arg(long, default_value_t = 50)]
    limit: u32,
}

/// The start of a day, in milliseconds since 1970.
fn parse_date(s: &str) -> Result<u64, String> {
    let format = time::macros::format_description!("[year]-[month]-[day]");
    let day = time::Date::parse(s, &format).map_err(|_| format!("not a date: {s} (YYYY-MM-DD)"))?;
    let secs = day.midnight().assume_utc().unix_timestamp();
    u64::try_from(secs * 1000).map_err(|_| "before 1970".into())
}

pub async fn timeline(g: &Global, a: TimelineArgs) -> Res {
    let s = g.server()?;
    let rows = s
        .timeline(&t::TimelineQuery {
            before: a.before,
            limit: Some(a.limit),
            ..t::TimelineQuery::default()
        })
        .await?;
    print_rows(g, &s, &rows).await
}

#[derive(Args)]
pub struct NearArgs {
    /// Latitude
    #[arg(allow_negative_numbers = true)]
    lat: f64,
    /// Longitude
    #[arg(allow_negative_numbers = true)]
    lon: f64,
    /// How far, in kilometres
    #[arg(long, default_value_t = 1.0)]
    km: f64,
    #[arg(long, default_value_t = 50)]
    limit: u32,
}

pub async fn near(g: &Global, a: NearArgs) -> Res {
    let s = g.server()?;
    let rows = s
        .near(&t::NearQuery {
            lat: a.lat,
            lon: a.lon,
            km: Some(a.km),
            limit: Some(a.limit),
        })
        .await?;
    print_rows(g, &s, &rows).await
}

#[derive(Args)]
pub struct RelatedArgs {
    /// The space (name or ID)
    #[arg(add = ArgValueCandidates::new(crate::complete::spaces))]
    space: String,
    /// TYPE/ID
    target: String,
}

pub async fn related(g: &Global, a: RelatedArgs) -> Res {
    let s = g.server()?;
    let list = s.related(&a.space, &a.target, &t::Limit::default()).await?;
    if g.json {
        print_json(&list);
        return Ok(());
    }
    let rows: Vec<Vec<String>> = list
        .iter()
        .map(|x| vec![x.rel.clone(), x.row.target.clone(), x.row.title.clone()])
        .collect();
    table(&["RELATION", "RESOURCE", "TITLE"], &rows);
    Ok(())
}

// Blobs and events

#[derive(Subcommand)]
pub enum BlobCommand {
    /// Store a file's bytes; prints their hash
    Put {
        file: PathBuf,
        #[arg(long, value_name = "TYPE")]
        media_type: Option<String>,
    },
    /// Fetch bytes by their hash
    Get {
        hash: String,
        /// Write them to this file
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,
    },
}

pub async fn blob(g: &Global, cmd: BlobCommand) -> Res {
    let s = g.server()?;
    match cmd {
        BlobCommand::Put { file, media_type } => {
            let bytes = read_input(Some(&file))?;
            let media_type = media_type
                .as_deref()
                .or_else(|| guess_media_type(Some(&file), &[]))
                .unwrap_or("application/octet-stream");
            let r = s.put_blob(media_type, bytes).await?;
            if g.json {
                print_json(&r);
            } else {
                println!("{}", r.hash);
            }
            Ok(())
        }
        BlobCommand::Get { hash, output } => {
            let path = format!("/api/blobs/{}", hs_api::client::seg(&hash));
            if let Some(p) = output {
                let f = std::fs::File::create(&p).map_err(|e| format!("{}: {e}", p.display()))?;
                let mut w = std::io::BufWriter::new(f);
                s.download(&path, &mut w).await?;
                return Ok(());
            }
            let out = std::io::stdout();
            if out.is_terminal() {
                return Err("redirect it to a file, or give -o FILE".into());
            }
            s.download(&path, &mut out.lock()).await?;
            Ok(())
        }
    }
}

/// Print changes as they happen, one line each, until interrupted.
pub async fn watch(g: &Global) -> Res {
    let s = g.server()?;
    let names = remote_names(&s).await;
    let json = g.json;
    let events = s.events(|ev| {
        if json {
            println!("{}", serde_json::to_string(&ev).unwrap_or_default());
        } else {
            println!(
                "{}  {:<8}  {}  {}",
                time(hs_node::now_ms()),
                ev.kind,
                name_of(&names, &ev.space),
                ev.target
            );
        }
        std::io::stdout().flush().is_ok()
    });
    tokio::select! {
        r = events => Ok(r?),
        _ = tokio::signal::ctrl_c() => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_and_dates() {
        assert_eq!(parse_duration("90m"), Ok(5_400_000));
        assert_eq!(parse_duration("30d"), Ok(2_592_000_000));
        assert!(parse_duration("3y").is_err());
        assert!(parse_duration("h").is_err());
        assert_eq!(parse_date("1970-01-02"), Ok(86_400_000));
        assert!(parse_date("2026-13-01").is_err());
    }

    #[test]
    fn media_types_come_from_names_or_json() {
        let p = |s: &str| Some(PathBuf::from(s));
        assert_eq!(
            guess_media_type(p("a.VCF").as_deref(), b""),
            Some("text/vcard")
        );
        assert_eq!(
            guess_media_type(None, b"{\"a\": 1}"),
            Some("application/json")
        );
        assert_eq!(guess_media_type(p("notes").as_deref(), b"hello"), None);
    }

    #[test]
    fn etags_are_quoted() {
        assert_eq!(quoted("abc"), "\"abc\"");
        assert_eq!(quoted("\"abc\""), "\"abc\"");
    }
}
