// SPDX-License-Identifier: AGPL-3.0-only

//! The derived index for homeserver.
//!
//! The signed space logs and content-addressed blobs are the truth. This crate
//! builds an SQLite index from them with `SQLx`: the current version of each
//! resource, the links between resources, full-text search, places and the
//! timeline. The index never goes out of the node. It can be deleted and
//! built again from the logs at any time. A new schema means a rebuild, never
//! a migration.

pub mod text;

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value as Json;
use sqlx_core::query::query;
use sqlx_core::query_scalar::query_scalar;
use sqlx_core::raw_sql::raw_sql;
use sqlx_core::row::Row as _;
use sqlx_sqlite::{
    SqliteConnectOptions, SqliteConnection, SqliteJournalMode, SqlitePool, SqlitePoolOptions,
    SqliteRow, SqliteSynchronous,
};

/// The name of this component.
pub const NAME: &str = env!("CARGO_PKG_NAME");

/// Make this one higher every time the schema or the extraction changes. Then
/// the index is built again.
pub const SCHEMA_VERSION: i64 = 1;

/// An index error.
#[derive(Debug)]
pub struct Error {
    source: Box<sqlx_core::Error>,
    backtrace: std::backtrace::Backtrace,
}

impl Error {
    /// Read the backtrace that was taken at the edge of the index.
    pub fn backtrace(&self) -> &std::backtrace::Backtrace {
        &self.backtrace
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.source.fmt(f)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.source)
    }
}

impl From<sqlx_core::Error> for Error {
    fn from(source: sqlx_core::Error) -> Self {
        Self {
            source: Box::new(source),
            backtrace: std::backtrace::Backtrace::capture(),
        }
    }
}

const SCHEMA: &str = "
CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE spaces(space TEXT PRIMARY KEY, name TEXT NOT NULL, seq INTEGER NOT NULL);
CREATE TABLE resources(
  id INTEGER PRIMARY KEY,
  space TEXT NOT NULL, target TEXT NOT NULL, type TEXT NOT NULL,
  payload TEXT, media_type TEXT NOT NULL, seq INTEGER NOT NULL,
  updated INTEGER NOT NULL, time INTEGER NOT NULL,
  title TEXT NOT NULL, body TEXT,
  UNIQUE(space, target));
CREATE INDEX resources_type ON resources(space, type, time);
CREATE INDEX resources_time ON resources(time);
CREATE TABLE links(src INTEGER NOT NULL, rel TEXT NOT NULL, dst_space TEXT NOT NULL, dst TEXT NOT NULL);
CREATE INDEX links_src ON links(src);
CREATE INDEX links_dst ON links(dst_space, dst);
CREATE VIRTUAL TABLE fts USING fts5(text, content='', contentless_delete=1, tokenize='unicode61');
CREATE VIRTUAL TABLE places USING rtree(id, min_lat, max_lat, min_lon, max_lon);
";

/// One resource in the index.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Row {
    /// The space (hex).
    pub space: String,
    /// `type/id`.
    pub target: String,
    /// The type.
    #[serde(rename = "type")]
    pub kind: String,
    /// Payload hash (hex). `None` if erased.
    pub payload: Option<String>,
    /// Media type.
    pub media_type: String,
    /// Sequence number of the last change.
    pub seq: u64,
    /// When it last changed (ms).
    pub updated: u64,
    /// The time it is about (ms): its own `time` field, or `updated`.
    pub time: u64,
    /// A title for lists.
    pub title: String,
    /// The JSON body, if it is JSON.
    pub body: Option<Json>,
}

/// A change to index.
pub struct Change<'a> {
    /// The space (hex).
    pub space: &'a str,
    /// Sequence number of the entry.
    pub seq: u64,
    /// The primary's time (ms).
    pub time: u64,
    /// `put`, `delete` or `erase`. Other kinds only move the space's seq forward.
    pub kind: &'a str,
    /// `type/id`.
    pub target: &'a str,
    /// Payload hash (hex).
    pub payload: Option<String>,
    /// Media type.
    pub media_type: &'a str,
    /// The payload bytes, if they are there.
    pub bytes: Option<&'a [u8]>,
}

impl std::fmt::Debug for Change<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Change")
            .field("seq", &self.seq)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// What [`Index::open`] found.
#[derive(Debug, PartialEq, Eq)]
pub enum Opened {
    /// The index could be used.
    Ready,
    /// The index was missing, damaged or from another schema, and is now
    /// empty. Replay every log into it.
    Rebuild,
}

/// The index: one writer connection and a pool of readers (WAL mode).
#[derive(Debug)]
pub struct Index {
    writer: SqlitePool,
    readers: SqlitePool,
}

// The columns of a [`Row`]. It is a macro so that every query stays a static string.
macro_rules! cols {
    () => {
        "r.space, r.target, r.type, r.payload, r.media_type, r.seq, r.updated, r.time, r.title, r.body"
    };
}

fn row_at(r: &SqliteRow, o: usize) -> Result<Row, Error> {
    let body: Option<String> = r.try_get(o + 9)?;
    Ok(Row {
        space: r.try_get(o)?,
        target: r.try_get(o + 1)?,
        kind: r.try_get(o + 2)?,
        payload: r.try_get(o + 3)?,
        media_type: r.try_get(o + 4)?,
        seq: r.try_get::<i64, _>(o + 5)? as u64,
        updated: r.try_get::<i64, _>(o + 6)? as u64,
        time: r.try_get::<i64, _>(o + 7)? as u64,
        title: r.try_get(o + 8)?,
        body: body.and_then(|b| serde_json::from_str(&b).ok()),
    })
}

fn rows(found: Vec<SqliteRow>) -> Result<Vec<Row>, Error> {
    found.iter().map(|r| row_at(r, 0)).collect()
}

fn options(path: &Path) -> Result<SqliteConnectOptions, Error> {
    Ok(
        SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5)),
    )
}

async fn schema_version(path: &Path) -> Option<String> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options(path).ok()?)
        .await
        .ok()?;
    let v = query_scalar::<_, String>("SELECT value FROM meta WHERE key = 'schema'")
        .fetch_optional(&pool)
        .await
        .ok()
        .flatten();
    pool.close().await;
    v
}

impl Index {
    /// Open the index at `path`. If it is missing, damaged or has another
    /// schema version, it is deleted and made again, empty.
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be opened or its schema cannot be set up.
    pub async fn open(path: &Path) -> Result<(Index, Opened), Error> {
        let usable = path.exists()
            && schema_version(path)
                .await
                .is_some_and(|v| v == SCHEMA_VERSION.to_string());
        let opened = if usable {
            Opened::Ready
        } else {
            Opened::Rebuild
        };
        if !usable {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
            }
        }
        let writer = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options(path)?)
            .await?;
        if !usable {
            raw_sql(SCHEMA).execute(&writer).await?;
            query("INSERT INTO meta VALUES ('schema', ?1)")
                .bind(SCHEMA_VERSION.to_string())
                .execute(&writer)
                .await?;
        }
        let readers = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options(path)?.read_only(true))
            .await?;
        Ok((Index { writer, readers }, opened))
    }

    /// Close the pools.
    pub async fn close(&self) {
        self.writer.close().await;
        self.readers.close().await;
    }

    /// The last sequence number of a space that is in the index.
    ///
    /// # Errors
    ///
    /// Returns an error if the index cannot run the query or decode its result.
    pub async fn space_seq(&self, space: &str) -> Result<Option<u64>, Error> {
        let n: Option<i64> = query_scalar("SELECT seq FROM spaces WHERE space = ?1")
            .bind(space)
            .fetch_optional(&self.writer)
            .await?;
        Ok(n.map(|n| n as u64))
    }

    /// Record a space and its name.
    ///
    /// # Errors
    ///
    /// Returns an error if the index cannot store the space metadata.
    pub async fn set_space(&self, space: &str, name: &str, seq: u64) -> Result<(), Error> {
        query("INSERT INTO spaces VALUES (?1, ?2, ?3) ON CONFLICT(space) DO UPDATE SET name = ?2, seq = ?3")
            .bind(space)
            .bind(name)
            .bind(seq as i64)
            .execute(&self.writer)
            .await?;
        Ok(())
    }

    /// Index one log entry.
    ///
    /// # Errors
    ///
    /// Returns an error if the index cannot complete the transaction.
    pub async fn apply(&self, ch: &Change<'_>) -> Result<(), Error> {
        let mut tx = self.writer.begin().await?;
        if matches!(ch.kind, "put" | "delete" | "erase") {
            let old: Option<i64> =
                query_scalar("SELECT id FROM resources WHERE space = ?1 AND target = ?2")
                    .bind(ch.space)
                    .bind(ch.target)
                    .fetch_optional(&mut *tx)
                    .await?;
            if let Some(id) = old {
                for sql in [
                    "DELETE FROM fts WHERE rowid = ?1",
                    "DELETE FROM places WHERE id = ?1",
                    "DELETE FROM links WHERE src = ?1",
                    "DELETE FROM resources WHERE id = ?1",
                ] {
                    query(sql).bind(id).execute(&mut *tx).await?;
                }
            }
            if ch.kind == "put" {
                insert(&mut tx, ch, old).await?;
            }
        }
        query("UPDATE spaces SET seq = ?2 WHERE space = ?1")
            .bind(ch.space)
            .bind(ch.seq as i64)
            .execute(&mut *tx)
            .await?;
        Ok(tx.commit().await?)
    }

    /// Get one resource.
    ///
    /// # Errors
    ///
    /// Returns an error if the index cannot query or decode the resource.
    pub async fn get(&self, space: &str, target: &str) -> Result<Option<Row>, Error> {
        let r = query(concat!(
            "SELECT ",
            cols!(),
            " FROM resources r WHERE space = ?1 AND target = ?2"
        ))
        .bind(space)
        .bind(target)
        .fetch_optional(&self.readers)
        .await?;
        r.map(|r| row_at(&r, 0)).transpose()
    }

    /// List resources of a type in a space, newest first.
    ///
    /// # Errors
    ///
    /// Returns an error if the index cannot query or decode the resources.
    pub async fn list(
        &self,
        space: &str,
        kind: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Row>, Error> {
        self.list_page(space, kind, limit, offset, None).await
    }

    /// List a page of resources of the kinds that are allowed.
    ///
    /// # Errors
    ///
    /// Returns an error if the index cannot query or decode the resources.
    pub async fn list_page(
        &self,
        space: &str,
        kind: Option<&str>,
        limit: u32,
        offset: u32,
        kinds: Option<&[String]>,
    ) -> Result<Vec<Row>, Error> {
        rows(query(concat!("SELECT ", cols!(), " FROM resources r WHERE space = ?1 AND (?2 IS NULL OR type = ?2)
          AND (?5 IS NULL OR type IN (SELECT value FROM json_each(?5))) ORDER BY time DESC, target LIMIT ?3 OFFSET ?4"))
        .bind(space)
        .bind(kind)
        .bind(limit)
        .bind(offset)
        .bind(kinds.map(|kinds| serde_json::to_string(kinds).unwrap_or_default()))
        .fetch_all(&self.readers)
        .await?)
    }

    /// Full-text search in the given spaces, best first.
    ///
    /// # Errors
    ///
    /// Returns an error if the index cannot run the full-text search or decode the results.
    pub async fn search(
        &self,
        input: &str,
        spaces: &[String],
        limit: u32,
    ) -> Result<Vec<Row>, Error> {
        let Some(q) = text::fts_query(input) else {
            return Ok(Vec::new());
        };
        rows(
            query(concat!(
                "SELECT ",
                cols!(),
                " FROM fts JOIN resources r ON r.id = fts.rowid
             WHERE fts MATCH ?1 AND r.space IN (SELECT value FROM json_each(?2))
             ORDER BY bm25(fts) LIMIT ?3"
            ))
            .bind(q)
            .bind(serde_json::to_string(spaces).unwrap_or_default())
            .bind(limit)
            .fetch_all(&self.readers)
            .await?,
        )
    }

    /// Resources with a place no more than `km` from a point, nearest first.
    ///
    /// # Errors
    ///
    /// Returns an error if the index cannot run the location query or decode the results.
    pub async fn near(
        &self,
        lat: f64,
        lon: f64,
        km: f64,
        spaces: &[String],
        limit: u32,
    ) -> Result<Vec<Row>, Error> {
        let dlat = km / 111.0;
        let dlon = km / (111.0 * lat.to_radians().cos().abs().max(0.01));
        rows(query(concat!("SELECT ", cols!(), " FROM places p JOIN resources r ON r.id = p.id
             WHERE p.max_lat >= ?1 AND p.min_lat <= ?2 AND p.max_lon >= ?3 AND p.min_lon <= ?4
               AND r.space IN (SELECT value FROM json_each(?5))
             ORDER BY (p.min_lat - ?6) * (p.min_lat - ?6) + (p.min_lon - ?7) * (p.min_lon - ?7) LIMIT ?8"))
        .bind(lat - dlat)
        .bind(lat + dlat)
        .bind(lon - dlon)
        .bind(lon + dlon)
        .bind(serde_json::to_string(spaces).unwrap_or_default())
        .bind(lat)
        .bind(lon)
        .bind(limit)
        .fetch_all(&self.readers)
        .await?)
    }

    /// The timeline: resources by the time they are about, newest first.
    ///
    /// # Errors
    ///
    /// Returns an error if the index cannot query or decode the timeline entries.
    pub async fn timeline(
        &self,
        spaces: &[String],
        before: Option<u64>,
        limit: u32,
    ) -> Result<Vec<Row>, Error> {
        self.timeline_page(spaces, before, limit, 0, None).await
    }

    /// Read a page of the timeline, in an order that does not change.
    ///
    /// # Errors
    ///
    /// Returns an error if the index cannot query or decode the timeline entries.
    pub async fn timeline_page(
        &self,
        spaces: &[String],
        before: Option<u64>,
        limit: u32,
        offset: u32,
        kinds: Option<&[String]>,
    ) -> Result<Vec<Row>, Error> {
        rows(query(concat!("SELECT ", cols!(), " FROM resources r WHERE r.space IN (SELECT value FROM json_each(?1)) AND r.time < ?2
             AND (?5 IS NULL OR r.type IN (SELECT value FROM json_each(?5)))
             AND r.type NOT IN ('grant', 'push', 'cipher', 'credential', 'oauth-client', 'vault-folder', 'vault-account') ORDER BY r.time DESC, r.space, r.target LIMIT ?3 OFFSET ?4"))
        .bind(serde_json::to_string(spaces).unwrap_or_default())
        .bind(before.and_then(|time| i64::try_from(time).ok()).unwrap_or(i64::MAX))
        .bind(limit)
        .bind(offset)
        .bind(kinds.map(|kinds| serde_json::to_string(kinds).unwrap_or_default()))
        .fetch_all(&self.readers)
        .await?)
    }

    /// Everything linked to or from a resource.
    ///
    /// # Errors
    ///
    /// Returns an error if the index cannot query or decode linked resources.
    pub async fn related(
        &self,
        space: &str,
        target: &str,
        limit: u32,
    ) -> Result<Vec<(String, Row)>, Error> {
        let found = query(concat!(
            "SELECT l.rel, ",
            cols!(),
            " FROM links l JOIN resources r ON r.id = l.src
               WHERE l.dst_space = ?1 AND l.dst = ?2
             UNION
             SELECT l.rel, ",
            cols!(),
            " FROM links l
               JOIN resources s ON s.id = l.src AND s.space = ?1 AND s.target = ?2
               JOIN resources r ON r.space = l.dst_space AND r.target = l.dst
             ORDER BY 9 DESC LIMIT ?3"
        ))
        .bind(space)
        .bind(target)
        .bind(limit)
        .fetch_all(&self.readers)
        .await?;
        found
            .iter()
            .map(|r| Ok((r.try_get(0)?, row_at(r, 1)?)))
            .collect()
    }

    /// Counts by type, for the health page and the home screen.
    ///
    /// # Errors
    ///
    /// Returns an error if the index cannot query or decode the type counts.
    pub async fn counts(&self) -> Result<Vec<(String, u64)>, Error> {
        let found = query("SELECT type, count(*) FROM resources GROUP BY type ORDER BY 2 DESC")
            .fetch_all(&self.readers)
            .await?;
        found
            .iter()
            .map(|r| Ok((r.try_get(0)?, r.try_get::<i64, _>(1)? as u64)))
            .collect()
    }

    /// Run SQLite's integrity check.
    pub async fn healthy(&self) -> bool {
        query_scalar::<_, String>("PRAGMA quick_check")
            .fetch_one(&self.readers)
            .await
            .is_ok_and(|s| s == "ok")
    }

    /// A canonical dump of everything derived. Tests use it to see that an
    /// index built step by step is the same as one built again from nothing.
    ///
    /// # Errors
    ///
    /// Returns an error if any index table cannot be queried or decoded.
    pub async fn dump(&self) -> Result<Vec<String>, Error> {
        let mut out = Vec::new();
        for r in query(concat!(
            "SELECT ",
            cols!(),
            " FROM resources r ORDER BY space, target"
        ))
        .fetch_all(&self.writer)
        .await?
        {
            out.push(format!("{:?}", row_at(&r, 0)?));
        }
        let links = query(
            "SELECT r.space, r.target, l.rel, l.dst_space, l.dst FROM links l JOIN resources r ON r.id = l.src ORDER BY 1, 2, 3, 4, 5",
        )
        .fetch_all(&self.writer)
        .await?;
        for r in links {
            let s = |i| r.try_get::<String, _>(i);
            out.push(format!(
                "link {}:{} {} {}:{}",
                s(0)?,
                s(1)?,
                s(2)?,
                s(3)?,
                s(4)?
            ));
        }
        for r in query("SELECT space, name, seq FROM spaces ORDER BY space")
            .fetch_all(&self.writer)
            .await?
        {
            out.push(format!(
                "space {} {} {}",
                r.try_get::<String, _>(0)?,
                r.try_get::<String, _>(1)?,
                r.try_get::<i64, _>(2)?
            ));
        }
        Ok(out)
    }

    #[cfg(test)]
    async fn exec(&self, sql: &'static str) -> Result<(), Error> {
        query(sql).execute(&self.writer).await?;
        Ok(())
    }
}

async fn insert(
    tx: &mut SqliteConnection,
    ch: &Change<'_>,
    reuse_id: Option<i64>,
) -> Result<(), Error> {
    let kind = ch.target.split('/').next().unwrap_or("");
    let json: Option<Json> = ch
        .bytes
        .filter(|_| {
            ch.media_type.starts_with("application/json") || ch.media_type.ends_with("+json")
        })
        .and_then(|b| serde_json::from_slice(b).ok());
    // vCard, iCalendar, Markdown and plain text are searchable too.
    let plain: Option<String> = ch
        .bytes
        .filter(|_| json.is_none() && ch.media_type.starts_with("text/"))
        .and_then(|b| std::str::from_utf8(b).ok())
        .map(String::from);
    let title = match (&json, &plain) {
        (Some(j), _) => text::title(j),
        (None, Some(t)) => text::text_title(t),
        _ => String::new(),
    };
    let time = json.as_ref().and_then(text::time_of).unwrap_or(ch.time);
    let id: i64 = query_scalar(
        "INSERT INTO resources(id, space, target, type, payload, media_type, seq, updated, time, title, body)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) RETURNING id",
    )
    .bind(reuse_id)
    .bind(ch.space)
    .bind(ch.target)
    .bind(kind)
    .bind(&ch.payload)
    .bind(ch.media_type)
    .bind(ch.seq as i64)
    .bind(ch.time as i64)
    .bind(time as i64)
    .bind(&title)
    .bind(json.as_ref().map(Json::to_string))
    .fetch_one(&mut *tx)
    .await?;
    // Secrets are never indexed for search.
    let secret = matches!(kind, "cipher" | "grant" | "push" | "credential");
    let body = match (&json, plain) {
        (Some(j), _) => text::searchable(j),
        (None, Some(t)) => t,
        _ => String::new(),
    };
    if !secret && !body.trim().is_empty() {
        query("INSERT INTO fts(rowid, text) VALUES (?1, ?2)")
            .bind(id)
            .bind(text::bigrams(&body))
            .execute(&mut *tx)
            .await?;
    }
    let Some(json) = json else { return Ok(()) };
    if let Some((lat, lon)) = text::place(&json) {
        query("INSERT INTO places VALUES (?1, ?2, ?2, ?3, ?3)")
            .bind(id)
            .bind(lat)
            .bind(lon)
            .execute(&mut *tx)
            .await?;
    }
    for (rel, dst_space, dst) in text::links(&json, ch.space) {
        query("INSERT INTO links VALUES (?1, ?2, ?3, ?4)")
            .bind(id)
            .bind(rel)
            .bind(dst_space)
            .bind(dst)
            .execute(&mut *tx)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change<'a>(
        space: &'a str,
        seq: u64,
        kind: &'a str,
        target: &'a str,
        body: Option<&'a [u8]>,
    ) -> Change<'a> {
        Change {
            space,
            seq,
            time: 1_000 * seq,
            kind,
            target,
            payload: body.map(|b| hs_core::Hash::of(b).to_hex()),
            media_type: "application/json",
            bytes: body,
        }
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("hs-index-{name}-{}.db", std::process::id()));
        for s in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{s}", p.display()));
        }
        p
    }

    #[tokio::test]
    async fn search_links_places_and_rebuild() {
        let path = temp("a");
        let (ix, opened) = Index::open(&path).await.unwrap();
        assert_eq!(opened, Opened::Rebuild);
        ix.set_space("s", "home", 0).await.unwrap();
        let mom = r#"{"name":"妈妈"}"#.as_bytes();
        let note = r#"{"title":"周末","text":"和妈妈去人民公园散步, a nice walk","time":"2024-05-01T09:30:00Z","place":{"lat":31.23,"lon":121.47},"links":[{"rel":"with","href":"person/mom"}]}"#.as_bytes();
        let vcard = "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:王小明\r\nEND:VCARD\r\n";
        let changes = [
            change("s", 1, "put", "person/mom", Some(mom)),
            change("s", 2, "put", "note/1", Some(note)),
            change(
                "s",
                3,
                "put",
                "note/2",
                Some(r#"{"text":"公司开会"}"#.as_bytes()),
            ),
            change("s", 4, "delete", "note/2", None),
            Change {
                media_type: "text/vcard",
                ..change("s", 5, "put", "contact/w", Some(vcard.as_bytes()))
            },
        ];
        for c in &changes {
            ix.apply(c).await.unwrap();
        }
        let spaces = vec!["s".to_string()];
        for q in ["公园", "妈妈", "散步", "人民公园", "walk", "Walk"] {
            let hits = ix.search(q, &spaces, 10).await.unwrap();
            assert!(hits.iter().any(|r| r.target == "note/1"), "{q}");
        }
        assert_eq!(
            ix.search("开会", &spaces, 10).await.unwrap(),
            [] as [Row; 0]
        );
        assert_eq!(
            ix.search("\"; DROP", &spaces, 10).await.unwrap(),
            [] as [Row; 0]
        );
        assert_eq!(
            ix.search("小明", &spaces, 10).await.unwrap()[0].title,
            "王小明"
        );
        assert_eq!(
            ix.near(31.2, 121.5, 10.0, &spaces, 10).await.unwrap().len(),
            1
        );
        assert_eq!(
            ix.near(40.0, 116.0, 10.0, &spaces, 10).await.unwrap(),
            [] as [Row; 0]
        );
        let related = ix.related("s", "person/mom", 10).await.unwrap();
        assert_eq!(related[0].1.target, "note/1");
        assert_eq!(
            ix.get("s", "note/1").await.unwrap().unwrap().time,
            1_714_555_800_000
        );
        assert_eq!(ix.timeline(&spaces, None, 10).await.unwrap().len(), 3);
        assert!(ix.healthy().await);

        // Built step by step == built again.
        let incremental = ix.dump().await.unwrap();
        let (ix2, _) = Index::open(&temp("b")).await.unwrap();
        ix2.set_space("s", "home", 0).await.unwrap();
        for c in &changes {
            ix2.apply(c).await.unwrap();
        }
        assert_eq!(ix2.dump().await.unwrap(), incremental);

        // Opening it again keeps it. A schema change builds it again.
        ix.close().await;
        let (ix, opened) = Index::open(&path).await.unwrap();
        assert_eq!(opened, Opened::Ready);
        ix.exec("UPDATE meta SET value = '0' WHERE key = 'schema'")
            .await
            .unwrap();
        ix.close().await;
        let (_, opened) = Index::open(&path).await.unwrap();
        assert_eq!(opened, Opened::Rebuild);
    }
}
