//! The MCP server list carries what the store knows about each server — its
//! description, installed version and registration time — so the dashboard
//! can draw a role beside every name without inventing one.

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use cloto_core::db;
use cloto_core::handlers::{get_mcp_server_tools, list_mcp_servers};
use cloto_core::managers::mcp_protocol::McpServerConfig;
use cloto_core::test_utils::create_test_app_state;
use cloto_core::{AppError, AppState};
use std::sync::Arc;

const API_KEY: &str = "test-key";

fn keyed() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-api-key", API_KEY.parse().unwrap());
    headers
}

async fn state() -> Arc<AppState> {
    create_test_app_state(Some(API_KEY.into())).await
}

/// Register a server whose command cannot start: it is saved and listed with
/// an error status, which is enough for the list to be drawn.
async fn register(s: &Arc<AppState>, id: &str, description: Option<&str>) {
    let config = McpServerConfig {
        id: id.to_string(),
        command: "/nonexistent/cloto-test-server".to_string(),
        ..Default::default()
    };
    let _ = s
        .mcp_manager
        .add_server_config(config, None, description.map(str::to_string))
        .await;
}

async fn listed(s: &Arc<AppState>) -> Vec<serde_json::Value> {
    let Ok(Json(body)) = list_mcp_servers(State(s.clone()), keyed()).await else {
        panic!("the list answers");
    };
    body["data"]["servers"].as_array().unwrap().clone()
}

#[tokio::test]
async fn the_list_carries_the_description_and_the_registration_time() {
    let s = state().await;
    register(&s, "described", Some("Reads the disks")).await;
    register(&s, "nameless", None).await;

    let servers = listed(&s).await;
    let described = servers
        .iter()
        .find(|v| v["id"] == "described")
        .expect("the registered server is listed even though it failed to start");
    assert_eq!(described["description"], "Reads the disks");
    let at = described["installed_at"]
        .as_i64()
        .expect("installed_at is the registration time");
    assert!(at > 1_700_000_000, "installed_at is unix seconds, got {at}");

    let nameless = servers.iter().find(|v| v["id"] == "nameless").unwrap();
    assert!(
        nameless.get("description").is_none(),
        "a server without a description carries none, not an empty string"
    );
}

#[tokio::test]
async fn a_blank_description_is_no_description() {
    let s = state().await;
    register(&s, "blank", Some("   ")).await;
    let servers = listed(&s).await;
    let blank = servers.iter().find(|v| v["id"] == "blank").unwrap();
    assert!(blank.get("description").is_none());
}

#[tokio::test]
async fn the_installed_version_comes_from_the_store() {
    let s = state().await;
    register(&s, "versioned", Some("Versioned")).await;
    db::set_marketplace_fields(&s.pool, "versioned", "2.5.12", "catalog-id", None, None)
        .await
        .unwrap();
    let servers = listed(&s).await;
    let v = servers.iter().find(|v| v["id"] == "versioned").unwrap();
    assert_eq!(v["installed_version"], "2.5.12");
}

#[tokio::test]
async fn the_tools_route_answers_for_a_registered_server_and_not_for_a_stranger() {
    let s = state().await;
    register(&s, "quiet", Some("Never started")).await;

    let Ok(Json(body)) =
        get_mcp_server_tools(State(s.clone()), keyed(), Path("quiet".to_string())).await
    else {
        panic!("a registered server answers");
    };
    assert_eq!(body["data"]["server_id"], "quiet");
    assert_eq!(
        body["data"]["tools"].as_array().map(Vec::len),
        Some(0),
        "a server that never started has no tools, and says so with an empty list"
    );

    let stranger = get_mcp_server_tools(State(s), keyed(), Path("stranger".to_string())).await;
    assert!(matches!(stranger, Err(AppError::Validation(_))));
}

/// The routes exist only if the kernel registers them; the handlers cannot
/// tell. Read out of the source that builds the router.
#[test]
fn the_kernel_registers_the_tools_route() {
    let wiring = include_str!("../../src/lib.rs");
    for needle in [
        "\"/mcp/servers/{name}/tools\"",
        "handlers::get_mcp_server_tools",
    ] {
        assert!(wiring.contains(needle), "{needle} is not wired in lib.rs");
    }
}
