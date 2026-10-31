//! The `objects` feature: manifests, channel objects, policies, grants, revocations, deposits,
//! join requests and gate tokens, stored in the log's database beside the entries.

pub(crate) mod handlers;
pub mod store;

pub use handlers::{GATE_DOMAIN, GATE_RETIRE_DOMAIN, REVOKE_DOMAIN};

use crate::server::State;
use axum::routing::{delete, get, post, put};
use axum::Router;

/// The object routes.
pub fn routes(router: Router<State>) -> Router<State> {
    use handlers::*;
    router
        .route(
            "/v1/circles/:circle/manifest",
            put(put_manifest).get(get_manifest),
        )
        .route("/v1/channels", post(post_channel))
        .route("/v1/channels/:channel", get(get_channel))
        .route("/v1/channels/:channel/policy", post(post_policy))
        .route(
            "/v1/channels/:channel/grants",
            post(post_grant).get(get_grants),
        )
        .route("/v1/channels/:channel/grants/:cti", get(get_grant))
        .route(
            "/v1/channels/:channel/revocations",
            post(post_revocation).get(get_revocations),
        )
        .route(
            "/v1/channels/:channel/deposits",
            post(post_deposit).get(get_deposits),
        )
        .route(
            "/v1/channels/:channel/requests",
            post(post_request).get(get_requests),
        )
        .route("/v1/channels/:channel/gates", post(post_gate))
        .route(
            "/v1/channels/:channel/gates/:token_hash",
            delete(delete_gate),
        )
}
