// SPDX-License-Identifier: AGPL-3.0-only

//! The `OpenAPI` 3.1 document of the REST API.
//!
//! It is built from the REST handlers (`rest.rs`) and the types in
//! [`crate::types`]. A node serves it at `/api/openapi.json`, `hs openapi`
//! prints it, and the repository keeps a copy in `docs/openapi.json`.

use std::sync::OnceLock;

use utoipa::OpenApi;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::openapi::{ContentBuilder, RefOr, ResponseBuilder};

use crate::types as t;
use crate::{push, rest};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "homeserver",
        description = "The REST API of a homeserver node. Protected operations accept a token: \
            `Authorization: Bearer …`, the password of HTTP Basic, or the session cookie \
            set by `POST /api/session`. Errors are `{\"error\": \"…\"}`.",
        license(name = "AGPL-3.0-only", identifier = "AGPL-3.0-only"),
    ),
    paths(
        rest::get_session, rest::sign_in, rest::sign_out,
        rest::node, rest::openapi, rest::identity, rest::health, rest::export,
        rest::list_spaces, rest::new_space, rest::join_space, rest::get_space,
        rest::add_member, rest::remove_member, rest::handover, rest::pull,
        rest::list_resources, rest::get_resource, rest::head_resource, rest::put_resource,
        rest::patch_resource, rest::delete, rest::erase,
        rest::search, rest::timeline, rest::near, rest::related,
        rest::put_blob, rest::get_blob, rest::head_blob, rest::events,
        rest::list_grants, rest::new_grant, rest::revoke_grant,
        rest::list_replicas, rest::sync_replicas,
        push::key, push::subscribe, push::test_notification,
    ),
    components(schemas(t::Event)),
    modifiers(&Common),
    security(("token" = [])),
    tags(
        (name = "session", description = "Signing in and out"),
        (name = "node", description = "The node itself"),
        (name = "spaces", description = "Spaces and their members"),
        (name = "resources", description = "What spaces hold"),
        (name = "find", description = "Search, the timeline, places and links"),
        (name = "blobs", description = "Bytes by their hash"),
        (name = "events", description = "Changes as they happen"),
        (name = "grants", description = "Tokens and what they may do"),
        (name = "push", description = "Browser push notifications"),
    ),
)]
struct Doc;

/// The bearer token scheme, and the error every operation may answer with.
struct Common;

impl utoipa::Modify for Common {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "token",
            SecurityScheme::Http(HttpBuilder::new().scheme(HttpAuthScheme::Bearer).build()),
        );
        let error = ResponseBuilder::new()
            .description("An error")
            .content(
                "application/json",
                ContentBuilder::new()
                    .schema(Some(RefOr::Ref(utoipa::openapi::Ref::from_schema_name(
                        "Error",
                    ))))
                    .build(),
            )
            .build();
        components
            .schemas
            .entry("Error".into())
            .or_insert_with(<t::Error as utoipa::PartialSchema>::schema);
        for item in openapi.paths.paths.values_mut() {
            for op in [
                &mut item.get,
                &mut item.put,
                &mut item.post,
                &mut item.delete,
                &mut item.patch,
                &mut item.head,
            ]
            .into_iter()
            .flatten()
            {
                op.responses
                    .responses
                    .entry("default".into())
                    .or_insert_with(|| RefOr::T(error.clone()));
            }
        }
    }
}

fn document() -> utoipa::openapi::OpenApi {
    let mut doc = Doc::openapi();
    doc.info.version = env!("CARGO_PKG_VERSION").to_string();
    doc
}

/// The document as pretty JSON, built once.
///
/// # Panics
///
/// Panics if the generated document violates JSON serialization invariants.
pub fn json() -> Vec<u8> {
    static JSON: OnceLock<Vec<u8>> = OnceLock::new();
    JSON.get_or_init(|| {
        let mut text = document()
            .to_pretty_json()
            .expect("the generated OpenAPI document must serialize as JSON");
        text.push('\n');
        text.into_bytes()
    })
    .clone()
}
