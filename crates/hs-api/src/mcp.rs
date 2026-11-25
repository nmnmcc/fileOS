// SPDX-License-Identifier: AGPL-3.0-only

//! Model Context Protocol over HTTP (`POST /mcp`), so an AI assistant the
//! owner chooses can read and write what a grant allows. The assistant gets
//! a token like any other app; this module adds no powers of its own.
//!
//! Answers are plain JSON responses (no SSE stream), which the Streamable
//! HTTP transport allows.

use hs_core::event::Kind;
use hs_node::Access;
use http::{Method, StatusCode};
use serde_json::{Value as Json, json};

use crate::util::{ApiError, Result, empty, json_ok};
use crate::{Api, rest, sync};
use hs_net::server::Request;

const PROTOCOL: &str = "2025-06-18";

fn tools() -> Json {
    let space = json!({ "type": "string", "description": "Space name or id" });
    let target =
        json!({ "type": "string", "description": "Resource, like note/2026-11-25 or contact/ana" });
    let limit = json!({ "type": "integer", "minimum": 1, "maximum": 200 });
    json!([
        { "name": "list_spaces", "description": "The spaces this token can read, with their names and ids.",
          "inputSchema": { "type": "object", "properties": {} } },
        { "name": "search", "description": "Full-text search across readable spaces, best first.",
          "inputSchema": { "type": "object", "properties": { "query": { "type": "string" }, "limit": limit }, "required": ["query"] } },
        { "name": "timeline", "description": "Resources by the time they are about, newest first.",
          "inputSchema": { "type": "object", "properties": { "before": { "type": "integer", "description": "Unix ms" }, "limit": limit } } },
        { "name": "near", "description": "Resources with a place near a point, nearest first.",
          "inputSchema": { "type": "object", "properties": { "lat": { "type": "number" }, "lon": { "type": "number" }, "km": { "type": "number" }, "limit": limit }, "required": ["lat", "lon"] } },
        { "name": "get", "description": "Read one resource.",
          "inputSchema": { "type": "object", "properties": { "space": space, "target": target }, "required": ["space", "target"] } },
        { "name": "related", "description": "Everything linked to or from a resource.",
          "inputSchema": { "type": "object", "properties": { "space": space, "target": target, "limit": limit }, "required": ["space", "target"] } },
        { "name": "list", "description": "Resources of one type in a space, newest first.",
          "inputSchema": { "type": "object", "properties": { "space": space, "type": { "type": "string" }, "limit": limit }, "required": ["space"] } },
        { "name": "put", "description": "Create or replace a JSON resource. Needs a writer token.",
          "inputSchema": { "type": "object", "properties": { "space": space, "target": target, "body": { "type": "object" } }, "required": ["space", "target", "body"] } },
    ])
}

fn text(v: &Json) -> Json {
    json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(v).unwrap_or_default() }], "structuredContent": { "result": v } })
}

fn arg<'a>(a: &'a Json, k: &str) -> std::result::Result<&'a str, String> {
    a.get(k)
        .and_then(Json::as_str)
        .ok_or_else(|| format!("{k} is required"))
}

fn limit(a: &Json) -> u32 {
    a.get("limit")
        .and_then(Json::as_u64)
        .map_or(20, |n| n.clamp(1, 200) as u32)
}

fn rows(access: &Access, rows: Vec<hs_store::Row>) -> Json {
    Json::Array(
        rows.into_iter()
            .filter(|r| rest::type_ok(access, &r.kind))
            .filter_map(|r| serde_json::to_value(r).ok())
            .collect(),
    )
}

async fn call(
    api: &Api,
    access: &Access,
    name: &str,
    a: &Json,
) -> std::result::Result<Json, String> {
    let spaces: Vec<String> = rest::readable(api, access)
        .iter()
        .map(hs_core::Hash::to_hex)
        .collect();
    let e = |e: ApiError| e.message;
    let s = |e: hs_store::Error| e.to_string();
    Ok(match name {
        "list_spaces" => {
            let mut out = Vec::new();
            for id in rest::readable(api, access) {
                out.push(rest::space_json(api, &id).await.map_err(e)?);
            }
            serde_json::to_value(out).unwrap_or_default()
        }
        "search" => rows(
            access,
            api.node
                .index
                .search(arg(a, "query")?, &spaces, limit(a))
                .await
                .map_err(s)?,
        ),
        "timeline" => {
            let before = a.get("before").and_then(Json::as_u64);
            rows(
                access,
                api.node
                    .index
                    .timeline(&spaces, before, limit(a))
                    .await
                    .map_err(s)?,
            )
        }
        "near" => {
            let n = |k: &str| {
                a.get(k)
                    .and_then(Json::as_f64)
                    .ok_or_else(|| format!("{k} is required"))
            };
            let km = a.get("km").and_then(Json::as_f64).unwrap_or(1.0);
            rows(
                access,
                api.node
                    .index
                    .near(n("lat")?, n("lon")?, km, &spaces, limit(a))
                    .await
                    .map_err(s)?,
            )
        }
        "list" => {
            let id = rest::space(api, access, arg(a, "space")?)
                .await
                .map_err(e)?;
            let kind = a.get("type").and_then(Json::as_str);
            rows(
                access,
                api.node
                    .index
                    .list(&id.to_hex(), kind, limit(a), 0)
                    .await
                    .map_err(s)?,
            )
        }
        "get" => {
            let id = rest::space(api, access, arg(a, "space")?)
                .await
                .map_err(e)?;
            let target = arg(a, "target")?;
            let kind = target.split('/').next().unwrap_or("");
            if !rest::type_ok(access, kind) {
                return Err("not found".into());
            }
            match api.node.index.get(&id.to_hex(), target).await.map_err(s)? {
                Some(r) => serde_json::to_value(r).unwrap_or_default(),
                None => return Err("not found".into()),
            }
        }
        "related" => {
            let id = rest::space(api, access, arg(a, "space")?)
                .await
                .map_err(e)?;
            let found = api
                .node
                .index
                .related(&id.to_hex(), arg(a, "target")?, limit(a))
                .await
                .map_err(s)?;
            Json::Array(
                found
                    .into_iter()
                    .filter(|(_, r)| rest::type_ok(access, &r.kind) && spaces.contains(&r.space))
                    .map(|(rel, r)| json!({ "rel": rel, "resource": r }))
                    .collect(),
            )
        }
        "put" => {
            let id = rest::space(api, access, arg(a, "space")?)
                .await
                .map_err(e)?;
            let target = arg(a, "target")?;
            let kind = target.split('/').next().unwrap_or("");
            if !rest::type_ok(access, kind) {
                return Err("this token cannot write that type".into());
            }
            let body = a
                .get("body")
                .filter(|b| b.is_object())
                .ok_or("body must be an object")?;
            let mut body = body.clone();
            if target.starts_with("file/") {
                rest::enrich_file(api, &mut body);
            }
            let bytes = serde_json::to_vec(&body).unwrap_or_default();
            let seq = sync::write(
                api,
                access,
                &id,
                Kind::Put,
                target,
                Some(&bytes),
                "application/json",
                None,
            )
            .await
            .map_err(e)?;
            json!({ "seq": seq })
        }
        _ => return Err(format!("unknown tool {name}")),
    })
}

fn reply(id: &Json, result: Json) -> Json {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: &Json, code: i64, message: &str) -> Json {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

async fn one(api: &Api, access: &Access, m: &Json) -> Option<Json> {
    let id = m.get("id")?.clone();
    let method = m.get("method").and_then(Json::as_str).unwrap_or("");
    let params = m.get("params").cloned().unwrap_or(Json::Null);
    Some(match method {
        "initialize" => reply(
            &id,
            json!({
                "protocolVersion": params.get("protocolVersion").and_then(Json::as_str).unwrap_or(PROTOCOL),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "homeserver", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "Personal data on the owner's homeserver. Resources are JSON, addressed as space + type/id.",
            }),
        ),
        "ping" => reply(&id, json!({})),
        "tools/list" => reply(&id, json!({ "tools": tools() })),
        "tools/call" => {
            let name = params.get("name").and_then(Json::as_str).unwrap_or("");
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            match call(api, access, name, &args).await {
                Ok(v) => reply(&id, text(&v)),
                Err(msg) => reply(
                    &id,
                    json!({ "content": [{ "type": "text", "text": msg }], "isError": true }),
                ),
            }
        }
        _ => error(&id, -32601, "method not found"),
    })
}

pub async fn handle(api: &Api, req: &Request, access: &Access) -> Result {
    if matches!(access, Access::Public) {
        return Err(ApiError::unauthorized());
    }
    if req.method() != Method::POST {
        return Err(ApiError::new(StatusCode::METHOD_NOT_ALLOWED, "only POST"));
    }
    let msg: Json = match serde_json::from_slice(req.body()) {
        Ok(v) => v,
        Err(_) => return Ok(json_ok(&error(&Json::Null, -32700, "parse error"))),
    };
    if let Some(batch) = msg.as_array() {
        let mut out = Vec::new();
        for m in batch {
            if let Some(r) = one(api, access, m).await {
                out.push(r);
            }
        }
        return Ok(if out.is_empty() {
            empty(StatusCode::ACCEPTED)
        } else {
            json_ok(&Json::Array(out))
        });
    }
    Ok(match one(api, access, &msg).await {
        Some(r) => json_ok(&r),
        None => empty(StatusCode::ACCEPTED),
    })
}
