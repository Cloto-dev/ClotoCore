use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use cloto_core::handlers;
use cloto_core::test_utils::create_test_app_state;
use cloto_core::AppState;
use serde_json::json;
use std::sync::Arc;
use tower::ServiceExt;

/// Helper function to create a test router with app state
fn create_test_router(state: Arc<AppState>) -> axum::Router {
    use axum::routing::{get, post, put};

    let admin_routes = axum::Router::new()
        .route("/agents", post(handlers::create_agent))
        .route("/agents/{id}", post(handlers::update_agent))
        .route(
            "/agents/{id}/mcp-access",
            put(handlers::put_agent_mcp_access),
        )
        .route("/cron/jobs", post(handlers::create_cron_job))
        .route("/plugins/{id}/config", post(handlers::update_plugin_config))
        .route(
            "/permissions/{id}/approve",
            post(handlers::approve_permission),
        )
        .route("/permissions/{id}/deny", post(handlers::deny_permission))
        // The tool-call surface. It is here rather than in `api_routes`
        // because it is an admin route in the kernel too: what makes it
        // special is the second credential it accepts, not its placement.
        .route("/mcp/call", post(handlers::call_mcp_tool))
        // Asset reads: authenticated by header or `?token=` (the browser
        // loads them through `<img src>`), never public.
        .route("/agents/{id}/avatar", get(handlers::get_avatar))
        .route("/agents/{id}/vrm", get(handlers::get_vrm))
        .route(
            "/chat/attachments/{attachment_id}",
            get(handlers::chat::get_attachment),
        );

    let api_routes = axum::Router::new()
        .route("/chat", post(handlers::chat_handler))
        .route("/agents", get(handlers::get_agents))
        .route("/plugins/{id}/config", get(handlers::get_plugin_config))
        .route("/llm/providers", get(handlers::list_llm_providers))
        // bug-475: register the pending-permissions read so its auth behavior
        // is locked in by tests (the handler calls check_auth).
        .route(
            "/permissions/pending",
            get(handlers::get_pending_permissions),
        )
        .merge(admin_routes)
        // A route whose handler performs NO auth check of its own. It stands
        // in for the next route someone adds and forgets to guard: the auth
        // layer below is what keeps it closed.
        .route("/unguarded", get(|| async { "open" }))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            cloto_core::middleware::auth_middleware,
        ))
        .with_state(state);

    axum::Router::new().nest("/api", api_routes)
}

/// The auth layer closes a route whose handler forgot `check_auth`.
#[tokio::test]
async fn auth_layer_denies_an_unguarded_route_without_a_key() {
    let state = create_test_app_state(Some("test-key".to_string())).await;
    let app = create_test_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/unguarded")
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("send request");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

// ---------------------------------------------------------------------------
// The agent-token path, measured through the layer rather than at the handler.
//
// `resolve_tool_caller` was correct and covered from the day it landed, and the
// feature still did not work: every request carrying only an agent token was
// refused by `auth_middleware` before the handler ran, because `/api/mcp/call`
// is not a public path. Calling the handler directly cannot see that — the
// layer is not in the picture — so these three go through the router.
//
// The observable has to be chosen with care. A *bogus* token is refused with
// the same 403 the layer returns, so it cannot tell "the handler declined" from
// "the handler never ran". A *resolvable* token can: the caller becomes an
// agent, the call proceeds, and it fails further in with a 400 for a tool no
// registered server provides. 400 here means "reached the handler".
// ---------------------------------------------------------------------------

const AGENT_TOKEN_HEADER: &str = "X-Agent-Token";

fn call_body() -> Body {
    Body::from(json!({"tool_name": "ping", "arguments": {}}).to_string())
}

/// A request carrying a resolvable agent token reaches the tool-call handler
/// without the admin key. This is the regression: it fails with 403 if
/// `/mcp/call` is not on the deferral list.
#[tokio::test]
async fn the_tool_call_route_admits_an_agent_token_without_the_admin_key() {
    let state = create_test_app_state(Some("test-key".to_string())).await;
    let token = state.agent_tokens.mint_default("agent.harness").await;
    let app = create_test_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/mcp/call")
                .header(header::CONTENT_TYPE, "application/json")
                .header(AGENT_TOKEN_HEADER, token)
                .body(call_body())
                .expect("build request"),
        )
        .await
        .expect("send request");

    assert_ne!(
        response.status(),
        StatusCode::FORBIDDEN,
        "the layer refused an agent token, so the handler never got to resolve it"
    );
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "expected the handler's own refusal (no server provides the tool)"
    );
}

/// The deferral is not an open door: with no credential at all, the layer still
/// closes the same route.
#[tokio::test]
async fn the_tool_call_route_stays_closed_with_no_credential() {
    let state = create_test_app_state(Some("test-key".to_string())).await;
    let app = create_test_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/mcp/call")
                .header(header::CONTENT_TYPE, "application/json")
                .body(call_body())
                .expect("build request"),
        )
        .await
        .expect("send request");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

/// With the layer standing aside, the handler's refusal is the only thing left
/// between an unresolvable token and the tool surface. A token that does not
/// resolve must not become the coordinator, which is what a fall-through to the
/// admin check would produce on a host where the admin key is also reachable.
#[tokio::test]
async fn an_unresolvable_agent_token_is_refused_rather_than_run_as_coordinator() {
    let state = create_test_app_state(Some("test-key".to_string())).await;
    let app = create_test_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/mcp/call")
                .header(header::CONTENT_TYPE, "application/json")
                .header(AGENT_TOKEN_HEADER, "not-a-token")
                .body(call_body())
                .expect("build request"),
        )
        .await
        .expect("send request");
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "an unresolvable token must be refused, not run as System"
    );
}

/// The deferral is scoped to the one route that can resolve a token. An agent
/// token is not a key to the rest of the admin surface.
#[tokio::test]
async fn an_agent_token_opens_nothing_but_the_tool_call_route() {
    let state = create_test_app_state(Some("test-key".to_string())).await;
    let token = state.agent_tokens.mint_default("agent.harness").await;
    let app = create_test_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/unguarded")
                .header(AGENT_TOKEN_HEADER, token)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("send request");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

/// The same route opens with the key in the header or as `?token=`; a wrong
/// token stays closed.
#[tokio::test]
async fn auth_layer_admits_a_valid_key_by_header_or_query() {
    let state = create_test_app_state(Some("test-key".to_string())).await;
    let app = create_test_router(state);

    let by_header = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/unguarded")
                .header("X-API-Key", "test-key")
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("send request");
    assert_eq!(by_header.status(), StatusCode::OK);

    let by_query = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/unguarded?token=test-key")
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("send request");
    assert_eq!(by_query.status(), StatusCode::OK);

    let wrong = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/unguarded?token=wrong")
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("send request");
    assert_eq!(wrong.status(), StatusCode::FORBIDDEN);
}

/// The public allowlist is exactly the bootstrap surface: liveness, version,
/// setup state and the two progress streams. Nothing else.
#[test]
fn public_api_allowlist_is_the_bootstrap_surface_only() {
    use cloto_core::middleware::{is_public_api_path, PUBLIC_API_PATHS};
    let expected = [
        "/system/version",
        "/system/health",
        "/setup/status",
        "/setup/progress",
        "/marketplace/progress",
    ];
    assert_eq!(PUBLIC_API_PATHS, &expected);
    for p in expected {
        assert!(is_public_api_path(p), "{p} should be public");
        assert!(
            is_public_api_path(&format!("/api{p}")),
            "/api{p} should be public"
        );
    }
    for p in [
        "/agents",
        "/system/shutdown",
        "/system/healthz",
        "/events",
        "/",
    ] {
        assert!(!is_public_api_path(p), "{p} must not be public");
    }
}

#[tokio::test]
async fn test_create_agent_success() {
    let state = create_test_app_state(Some("test-key".to_string())).await;
    let app = create_test_router(state);

    let payload = json!({
        "name": "Test Agent",
        "description": "A test agent",
        "default_engine": "mind.deepseek"
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agents")
                .header(header::CONTENT_TYPE, "application/json")
                .header("X-API-Key", "test-key")
                .body(Body::from(
                    serde_json::to_string(&payload).expect("serialize JSON"),
                ))
                .expect("build request"),
        )
        .await
        .expect("send request");

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_create_agent_invalid_payload() {
    let state = create_test_app_state(Some("test-key".to_string())).await;
    let app = create_test_router(state);

    // Missing required fields
    let payload = json!({
        "name": "Test Agent"
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agents")
                .header(header::CONTENT_TYPE, "application/json")
                .header("X-API-Key", "test-key")
                .body(Body::from(
                    serde_json::to_string(&payload).expect("serialize JSON"),
                ))
                .expect("build request"),
        )
        .await
        .expect("send request");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn test_update_plugin_config_success() {
    let state = create_test_app_state(Some("test-key".to_string())).await;

    // Insert a test plugin config first
    sqlx::query(
        "INSERT INTO plugin_configs (plugin_id, config_key, config_value) VALUES (?, ?, ?)",
    )
    .bind("test.plugin")
    .bind("api_key")
    .bind("old_value")
    .execute(&state.pool)
    .await
    .expect("insert test plugin config");

    let app = create_test_router(state);

    let payload = json!({
        "key": "api_key",
        "value": "new_value"
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/plugins/test.plugin/config")
                .header(header::CONTENT_TYPE, "application/json")
                .header("X-API-Key", "test-key")
                .body(Body::from(
                    serde_json::to_string(&payload).expect("serialize JSON"),
                ))
                .expect("build request"),
        )
        .await
        .expect("send request");

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_update_plugin_config_nonexistent_plugin() {
    let state = create_test_app_state(Some("test-key".to_string())).await;
    let app = create_test_router(state);

    let payload = json!({
        "key": "api_key",
        "value": "value"
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/plugins/nonexistent/config")
                .header(header::CONTENT_TYPE, "application/json")
                .header("X-API-Key", "test-key")
                .body(Body::from(
                    serde_json::to_string(&payload).expect("serialize JSON"),
                ))
                .expect("build request"),
        )
        .await
        .expect("send request");

    // Should succeed even if plugin doesn't exist (creates new config)
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_chat_handler_routes_to_agent() {
    // H-01: CLOTO_DEBUG_SKIP_AUTH required to bypass auth when no API key configured
    std::env::set_var("CLOTO_DEBUG_SKIP_AUTH", "1");
    let state = create_test_app_state(None).await;

    // Create a test agent first
    sqlx::query("INSERT INTO agents (id, name, description, status, default_engine_id, metadata) VALUES (?, ?, ?, ?, ?, ?)")
        .bind("agent.test")
        .bind("Test Agent")
        .bind("Test")
        .bind("active")
        .bind("mind.deepseek")
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert test agent");

    let app = create_test_router(state);

    let payload = json!({
        "id": "msg-123",
        "source": {
            "type": "User",
            "id": "user-1",
            "name": "Test User"
        },
        "target_agent": "agent.test",
        "content": "Hello, agent!",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "metadata": {}
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/chat")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(&payload).expect("serialize JSON"),
                ))
                .expect("build request"),
        )
        .await
        .expect("send request");

    // Chat handler should accept the request (or fail gracefully with 500 due to event channel issues in test)
    // In test environment, event_tx channel may not have receiver, causing send failure
    assert!(
        response.status() == StatusCode::OK
            || response.status() == StatusCode::INTERNAL_SERVER_ERROR
    );
}

#[tokio::test]
async fn test_grant_permission_requires_auth() {
    let state = create_test_app_state(Some("secret-key".to_string())).await;
    let app = create_test_router(state);

    let payload = json!({
        "approved_by": "admin"
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/permissions/test-id/approve")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(&payload).expect("serialize JSON"),
                ))
                .expect("build request"),
        )
        .await
        .expect("send request");

    // PermissionDenied maps to 403 Forbidden
    assert!(
        response.status() == StatusCode::FORBIDDEN
            || response.status() == StatusCode::UNAUTHORIZED
            || response.status() == StatusCode::BAD_REQUEST
    );
}

/// Insert a minimal `mcp_servers` row so that `mcp_access_control.server_id`
/// foreign-key references resolve during tests.
async fn seed_mcp_server(pool: &sqlx::SqlitePool, name: &str) {
    sqlx::query(
        "INSERT OR IGNORE INTO mcp_servers (name, command, args, created_at) \
         VALUES (?, 'python', '[]', strftime('%s', 'now'))",
    )
    .bind(name)
    .execute(pool)
    .await
    .expect("seed mcp_servers");
}

#[tokio::test]
async fn test_put_agent_mcp_access_replaces_grants() {
    let state = create_test_app_state(Some("test-key".to_string())).await;
    seed_mcp_server(&state.pool, "terminal").await;
    seed_mcp_server(&state.pool, "cpersona").await;
    seed_mcp_server(&state.pool, "mind.deepseek").await;

    // Pre-existing grant that should be removed by the replacement.
    sqlx::query(
        "INSERT INTO mcp_access_control \
         (entry_type, agent_id, server_id, permission, granted_at) \
         VALUES ('server_grant', 'agent.alice', 'mind.deepseek', 'allow', ?)",
    )
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(&state.pool)
    .await
    .expect("seed existing grant");

    let app = create_test_router(state.clone());

    let payload = json!({
        "granted_server_ids": ["terminal", "cpersona"]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/agents/agent.alice/mcp-access")
                .header(header::CONTENT_TYPE, "application/json")
                .header("X-API-Key", "test-key")
                .body(Body::from(
                    serde_json::to_string(&payload).expect("serialize JSON"),
                ))
                .expect("build request"),
        )
        .await
        .expect("send request");

    assert_eq!(response.status(), StatusCode::OK);

    // Verify: exactly the two new grants exist, the old deepseek grant is gone.
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT server_id FROM mcp_access_control \
         WHERE agent_id = 'agent.alice' AND entry_type = 'server_grant' \
         ORDER BY server_id",
    )
    .fetch_all(&state.pool)
    .await
    .expect("query grants");

    let server_ids: Vec<String> = rows.into_iter().map(|(s,)| s).collect();
    assert_eq!(server_ids, vec!["cpersona", "terminal"]);
}

#[tokio::test]
async fn test_put_agent_mcp_access_preserves_tool_grants() {
    let state = create_test_app_state(Some("test-key".to_string())).await;
    seed_mcp_server(&state.pool, "terminal").await;

    // Pre-existing tool_grant and capability for the agent — must survive.
    sqlx::query(
        "INSERT INTO mcp_access_control \
         (entry_type, agent_id, server_id, tool_name, permission, granted_at) \
         VALUES ('tool_grant', 'agent.bob', 'terminal', 'run_command', 'allow', ?)",
    )
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(&state.pool)
    .await
    .expect("seed tool_grant");

    sqlx::query(
        "INSERT INTO mcp_access_control \
         (entry_type, agent_id, server_id, permission, granted_at) \
         VALUES ('capability', 'agent.bob', 'terminal', 'allow', ?)",
    )
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(&state.pool)
    .await
    .expect("seed capability");

    let app = create_test_router(state.clone());

    // Replace server_grants with an empty set — other entry types must remain.
    let payload = json!({ "granted_server_ids": [] });

    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/agents/agent.bob/mcp-access")
                .header(header::CONTENT_TYPE, "application/json")
                .header("X-API-Key", "test-key")
                .body(Body::from(
                    serde_json::to_string(&payload).expect("serialize JSON"),
                ))
                .expect("build request"),
        )
        .await
        .expect("send request");

    assert_eq!(response.status(), StatusCode::OK);

    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT entry_type FROM mcp_access_control \
         WHERE agent_id = 'agent.bob' ORDER BY entry_type",
    )
    .fetch_all(&state.pool)
    .await
    .expect("query entries");

    let entry_types: Vec<String> = rows.into_iter().map(|(s,)| s).collect();
    assert_eq!(entry_types, vec!["capability", "tool_grant"]);
}

#[tokio::test]
async fn test_put_agent_mcp_access_auto_creates_missing_server_row() {
    // SetupWizard applies the preset before marketplace batch-install, so the
    // target servers may not exist in `mcp_servers` yet. The endpoint must
    // handle that by inserting a `config-loaded` placeholder (cleaned up later
    // by the real install via UPSERT).
    let state = create_test_app_state(Some("test-key".to_string())).await;
    // Intentionally do NOT pre-seed any mcp_servers rows.

    let app = create_test_router(state.clone());

    let payload = json!({
        "granted_server_ids": ["terminal", "cpersona"]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/agents/agent.setup/mcp-access")
                .header(header::CONTENT_TYPE, "application/json")
                .header("X-API-Key", "test-key")
                .body(Body::from(
                    serde_json::to_string(&payload).expect("serialize JSON"),
                ))
                .expect("build request"),
        )
        .await
        .expect("send request");

    assert_eq!(response.status(), StatusCode::OK);

    // Grants were inserted AND the referenced servers now exist as
    // config-loaded placeholders.
    let server_rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT name, command FROM mcp_servers \
         WHERE name IN ('terminal', 'cpersona') ORDER BY name",
    )
    .fetch_all(&state.pool)
    .await
    .expect("query mcp_servers");
    assert_eq!(server_rows.len(), 2);
    assert!(server_rows.iter().all(|(_, cmd)| cmd == "config-loaded"));

    let grant_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM mcp_access_control \
         WHERE agent_id = 'agent.setup' AND entry_type = 'server_grant'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("query grants");
    assert_eq!(grant_count.0, 2);
}

#[tokio::test]
async fn test_create_cron_job_unknown_agent_returns_validation_error() {
    // Regression: creating a CRON job for a non-existent agent_id used to
    // hit the cron_jobs.agent_id foreign-key and bubble up as a 500, which
    // LLM-driven agents interpreted as "CRON system is broken" and gave up.
    // The handler now pre-checks existence and returns a 400 with a
    // discovery hint.
    let state = create_test_app_state(Some("test-key".to_string())).await;
    let app = create_test_router(state);

    let payload = json!({
        "agent_id": "agent.does_not_exist",
        "name": "Test Job",
        "schedule_type": "interval",
        "schedule_value": "60",
        "message": "hello"
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/cron/jobs")
                .header(header::CONTENT_TYPE, "application/json")
                .header("X-API-Key", "test-key")
                .body(Body::from(
                    serde_json::to_string(&payload).expect("serialize JSON"),
                ))
                .expect("build request"),
        )
        .await
        .expect("send request");

    // AppError::Validation surfaces as 400 Bad Request.
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body_bytes = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .expect("read body");
    let body_str = std::str::from_utf8(&body_bytes).expect("utf8");
    assert!(
        body_str.contains("Unknown agent_id") && body_str.contains("mgp.discovery.list"),
        "expected validation message with discovery hint, got: {}",
        body_str
    );
}

#[tokio::test]
async fn test_put_agent_mcp_access_requires_auth() {
    let state = create_test_app_state(Some("secret-key".to_string())).await;
    let app = create_test_router(state);

    let payload = json!({ "granted_server_ids": [] });

    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/agents/agent.any/mcp-access")
                .header(header::CONTENT_TYPE, "application/json")
                // No X-API-Key header.
                .body(Body::from(
                    serde_json::to_string(&payload).expect("serialize JSON"),
                ))
                .expect("build request"),
        )
        .await
        .expect("send request");

    assert!(
        response.status() == StatusCode::FORBIDDEN
            || response.status() == StatusCode::UNAUTHORIZED
            || response.status() == StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn test_grant_permission_success() {
    let state = create_test_app_state(Some("test-key".to_string())).await;

    // Insert a pending permission request
    sqlx::query("INSERT INTO permission_requests (request_id, plugin_id, permission_type, justification, status, created_at) VALUES (?, ?, ?, ?, ?, ?)")
        .bind("req-123")
        .bind("test.plugin")
        .bind("NetworkAccess")
        .bind("Testing")
        .bind("pending")
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&state.pool)
        .await
        .expect("insert test permission request");

    let app = create_test_router(state);

    let payload = json!({
        "approved_by": "admin"
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/permissions/req-123/approve")
                .header(header::CONTENT_TYPE, "application/json")
                .header("X-API-Key", "test-key")
                .body(Body::from(
                    serde_json::to_string(&payload).expect("serialize JSON"),
                ))
                .expect("build request"),
        )
        .await
        .expect("send request");

    assert_eq!(response.status(), StatusCode::OK);
}

/// bug-475: deny_permission is auth-gated like approve — assert it rejects a
/// request with no X-API-Key (locks in the default-deny of the mutating path).
#[tokio::test]
async fn test_deny_permission_requires_auth() {
    let state = create_test_app_state(Some("test-key".to_string())).await;
    let app = create_test_router(state);

    let payload = json!({ "denied_by": "admin" });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/permissions/test-id/deny")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(&payload).expect("serialize JSON"),
                ))
                .expect("build request"),
        )
        .await
        .expect("send request");

    assert!(
        response.status() == StatusCode::FORBIDDEN
            || response.status() == StatusCode::UNAUTHORIZED
            || response.status() == StatusCode::BAD_REQUEST
    );
}

/// bug-475: deny_permission happy path — with a valid key and a pending row it
/// transitions the request to denied and returns 200.
#[tokio::test]
async fn test_deny_permission_success() {
    let state = create_test_app_state(Some("test-key".to_string())).await;

    sqlx::query("INSERT INTO permission_requests (request_id, plugin_id, permission_type, justification, status, created_at) VALUES (?, ?, ?, ?, ?, ?)")
        .bind("req-deny-1")
        .bind("test.plugin")
        .bind("NetworkAccess")
        .bind("Testing")
        .bind("pending")
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&state.pool)
        .await
        .expect("insert test permission request");

    let app = create_test_router(state);

    let payload = json!({ "denied_by": "admin" });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/permissions/req-deny-1/deny")
                .header(header::CONTENT_TYPE, "application/json")
                .header("X-API-Key", "test-key")
                .body(Body::from(
                    serde_json::to_string(&payload).expect("serialize JSON"),
                ))
                .expect("build request"),
        )
        .await
        .expect("send request");

    assert_eq!(response.status(), StatusCode::OK);
}

/// bug-475: lock in get_pending_permissions' actual behavior. The handler calls
/// check_auth, so a request with no X-API-Key must be rejected — this guards the
/// read endpoint's auth gate the same way the mutating endpoints are guarded.
#[tokio::test]
async fn test_pending_permissions_requires_auth() {
    let state = create_test_app_state(Some("test-key".to_string())).await;
    let app = create_test_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/permissions/pending")
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("send request");

    assert!(
        response.status() == StatusCode::FORBIDDEN
            || response.status() == StatusCode::UNAUTHORIZED
            || response.status() == StatusCode::BAD_REQUEST
    );
}

/// bug-475: get_pending_permissions happy path — with a valid key it returns 200.
#[tokio::test]
async fn test_pending_permissions_success() {
    let state = create_test_app_state(Some("test-key".to_string())).await;
    let app = create_test_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/permissions/pending")
                .header("X-API-Key", "test-key")
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("send request");

    assert_eq!(response.status(), StatusCode::OK);
}

/// GET /api/llm/providers annotates each provider with its backing-engine
/// state so the dashboard shows only real engines and warns (never drops) when
/// an engine is uninstalled. No engine MCP server is registered in
/// this harness, so classification is driven purely by user configuration.
#[tokio::test]
async fn test_llm_providers_engine_status_annotation() {
    async fn fetch(app: &axum::Router) -> serde_json::Value {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/llm/providers")
                    .header("X-API-Key", "test-key")
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("send request");
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("read body");
        serde_json::from_slice(&bytes).expect("parse JSON")
    }

    let state = create_test_app_state(Some("test-key".to_string())).await;
    let pool = state.pool.clone();
    let app = create_test_router(state);

    // Fresh install: the pristine-seed cleanup migration leaves llm_providers
    // empty — rows exist only after catalog ingest at engine install time.
    let body = fetch(&app).await;
    let providers = body["data"]["providers"]
        .as_array()
        .expect("providers array")
        .clone();
    assert!(
        providers.is_empty(),
        "fresh install has no provider rows (pristine seeds cleaned up)"
    );

    // Catalog ingest (as a marketplace install would run) creates the rows.
    // Neither engine server is registered and nothing is configured yet →
    // both are `catalog_only` (hidden by the dashboard). Placeholder metadata
    // rides along so the frontend keeps no hardcoded provider list of its own.
    for (id, name, url) in [
        (
            "deepseek",
            "DeepSeek",
            "https://api.deepseek.com/chat/completions",
        ),
        (
            "cerebras",
            "Cerebras",
            "https://api.cerebras.ai/v1/chat/completions",
        ),
    ] {
        cloto_core::db::upsert_llm_provider_meta(
            &pool,
            id,
            name,
            url,
            "bearer",
            "default-model",
            120,
            Some(r#"{"model_placeholder":"org/example-model"}"#.to_string()),
        )
        .await
        .expect("ingest provider meta");
    }
    let body = fetch(&app).await;
    let providers = body["data"]["providers"]
        .as_array()
        .expect("providers array")
        .clone();
    assert_eq!(providers.len(), 2);
    for p in &providers {
        assert_eq!(
            p["engine_status"], "catalog_only",
            "engineless + unconfigured provider {} must be catalog_only",
            p["id"]
        );
        assert_eq!(p["configured"], false);
        assert_eq!(p["model_placeholder"], "org/example-model");
    }

    // Configuring a provider's key (no engine installed) flips it to
    // `uninstalled` — kept and warned, not hidden — while the rest stay hidden.
    cloto_core::db::set_llm_provider_key(&pool, "deepseek", "sk-test")
        .await
        .expect("set provider key");
    let body = fetch(&app).await;
    let providers = body["data"]["providers"].as_array().unwrap();
    let deepseek = providers
        .iter()
        .find(|p| p["id"] == "deepseek")
        .expect("deepseek row");
    assert_eq!(deepseek["engine_status"], "uninstalled");
    assert_eq!(deepseek["configured"], true);
    assert_eq!(deepseek["has_key"], true);
    let cerebras = providers
        .iter()
        .find(|p| p["id"] == "cerebras")
        .expect("cerebras row");
    assert_eq!(cerebras["engine_status"], "catalog_only");
}

/// The pristine-seed cleanup migration deletes only rows that carry zero
/// user intent AND have no installed engine. Re-running its exact SQL (via
/// `include_str!`, so the test cannot drift from the shipped file) against
/// hand-built rows proves the two keep-conditions: user-configured rows and
/// rows backing a registered engine survive.
#[tokio::test]
async fn test_pristine_seed_cleanup_preserves_configured_and_installed_rows() {
    let state = create_test_app_state(Some("test-key".to_string())).await;
    let pool = state.pool.clone();

    // Rebuild three seed-shaped rows the way the (now-frozen) seed
    // migrations left them.
    for (id, model) in [
        ("claude", "claude-sonnet-4-6"),
        ("groq", "openai/gpt-oss-120b"),
        ("ollama", ""),
    ] {
        sqlx::query(
            "INSERT INTO llm_providers (id, display_name, api_url, model_id)
             VALUES (?, ?, 'https://example.com', ?)",
        )
        .bind(id)
        .bind(id)
        .bind(model)
        .execute(&pool)
        .await
        .expect("insert seed-shaped row");
    }
    // groq: user configured a key. ollama: engine is installed.
    cloto_core::db::set_llm_provider_key(&pool, "groq", "sk-user")
        .await
        .expect("set key");
    seed_mcp_server(&pool, "ollama").await;

    sqlx::raw_sql(include_str!(
        "../migrations/20260704000000_cleanup_pristine_llm_provider_seeds.sql"
    ))
    .execute(&pool)
    .await
    .expect("re-run cleanup migration SQL");

    let remaining: Vec<String> = sqlx::query_scalar("SELECT id FROM llm_providers ORDER BY id")
        .fetch_all(&pool)
        .await
        .expect("list remaining");
    assert_eq!(
        remaining,
        vec!["groq".to_string(), "ollama".to_string()],
        "pristine 'claude' deleted; configured 'groq' and installed 'ollama' kept"
    );
}

/// Ingesting provider metadata from a catalog entry updates only
/// provider-authored columns and never clobbers user-owned settings; the
/// default model is seeded only when the row is first created.
#[tokio::test]
async fn test_upsert_llm_provider_meta_preserves_user_columns() {
    let state = create_test_app_state(Some("test-key".to_string())).await;
    let pool = state.pool.clone();

    // Install-time ingest creates the row (fresh installs have none), then
    // the user applies their own settings on top.
    cloto_core::db::upsert_llm_provider_meta(
        &pool,
        "deepseek",
        "DeepSeek",
        "https://api.deepseek.com/chat/completions",
        "bearer",
        "deepseek-chat",
        120,
        None,
    )
    .await
    .expect("initial ingest");
    cloto_core::db::set_llm_provider_key(&pool, "deepseek", "sk-user")
        .await
        .expect("set key");
    sqlx::query("UPDATE llm_providers SET model_id = ?, thinking_mode = ? WHERE id = ?")
        .bind("user-chosen-model")
        .bind("on")
        .bind("deepseek")
        .execute(&pool)
        .await
        .expect("apply user model + thinking mode");

    // Re-ingest metadata (as a reinstall would): different api_url/auth/default.
    cloto_core::db::upsert_llm_provider_meta(
        &pool,
        "deepseek",
        "DeepSeek Renamed",
        "https://new.example.com/v1/chat/completions",
        "x-api-key",
        "ingested-default-model",
        99,
        Some(r#"{"model_placeholder":"org/example"}"#.to_string()),
    )
    .await
    .expect("upsert meta");

    let row = cloto_core::db::get_llm_provider(&pool, "deepseek")
        .await
        .expect("get deepseek");
    // Meta columns updated…
    assert_eq!(row.api_url, "https://new.example.com/v1/chat/completions");
    assert_eq!(row.auth_type, "x-api-key");
    assert_eq!(row.display_name, "DeepSeek Renamed");
    assert_eq!(row.timeout_secs, 99);
    assert_eq!(
        row.quirks_parsed().model_placeholder.as_deref(),
        Some("org/example")
    );
    // …user columns untouched.
    assert_eq!(row.api_key, "sk-user");
    assert_eq!(row.model_id, "user-chosen-model");
    assert_eq!(row.thinking_mode, "on");

    // Brand-new engine id: the row is created and the default model is seeded.
    cloto_core::db::upsert_llm_provider_meta(
        &pool,
        "newengine",
        "New Engine",
        "https://n.example.com/v1/chat/completions",
        "bearer",
        "seeded-default",
        120,
        None,
    )
    .await
    .expect("upsert new");
    let fresh = cloto_core::db::get_llm_provider(&pool, "newengine")
        .await
        .expect("get newengine");
    assert_eq!(fresh.model_id, "seeded-default");
    assert_eq!(fresh.api_key, "");
}

/// Every asset read (avatar, VRM, chat attachment) is denied without a key:
/// a headless kernel must not hand user content to whoever reaches the port.
#[tokio::test]
async fn asset_reads_require_auth() {
    let state = create_test_app_state(Some("test-key".to_string())).await;
    let app = create_test_router(state);

    for uri in [
        "/api/agents/agent.any/avatar",
        "/api/agents/agent.any/vrm",
        "/api/chat/attachments/att-any",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(uri)
                    // No X-API-Key header, no ?token=.
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("send request");
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "{uri} must be denied without a key"
        );
    }
}

/// The same reads accept the key as `?token=`, which is the only way an
/// `<img src>` / `<audio src>` / VRM loader can present it. With a valid
/// token the request gets past auth and fails on the missing asset instead.
#[tokio::test]
async fn asset_reads_accept_query_token() {
    let state = create_test_app_state(Some("test-key".to_string())).await;
    let app = create_test_router(state);

    for uri in [
        "/api/agents/agent.any/avatar?token=test-key",
        "/api/agents/agent.any/vrm?token=test-key",
        "/api/chat/attachments/att-any?token=test-key",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(uri)
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("send request");
        assert_ne!(
            response.status(),
            StatusCode::FORBIDDEN,
            "{uri} must pass auth with a query token"
        );
        // The asset does not exist, so anything past auth is an error for
        // the asset itself (the exact code is the handler's business).
        assert!(
            !response.status().is_success(),
            "{uri}: expected a missing-asset error after auth, got {}",
            response.status()
        );
    }
}

/// A wrong query token is not a bypass.
#[tokio::test]
async fn asset_reads_reject_wrong_query_token() {
    let state = create_test_app_state(Some("test-key".to_string())).await;
    let app = create_test_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/agents/agent.any/avatar?token=not-the-key")
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("send request");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

// ---------------------------------------------------------------------------
// Browser sessions, measured through the layer for the same reason the
// agent-token path is: the layer is not the only thing that checks. Handlers
// keep 97 `check_auth` calls of their own and those read the header alone, so a
// credential the layer understood by itself would pass the layer and be refused
// one frame later. Calling a handler directly cannot see that.
// ---------------------------------------------------------------------------

/// The test whose failure the whole design of the session branch is shaped
/// around: `get_agents` calls `check_auth` itself, so a cookie that only the
/// layer understood would get a 403 from the handler with the layer having said
/// yes. If this passes, the substitution reached the handler.
#[tokio::test]
async fn a_session_cookie_authenticates_a_route_that_checks_auth_itself() {
    let state = create_test_app_state(Some("test-key".to_string())).await;
    let token = state.browser_sessions.mint_default("operator").await;
    let app = create_test_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/agents")
                .header(header::COOKIE, format!("cloto_session={token}"))
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("send request");
    assert_ne!(
        response.status(),
        StatusCode::FORBIDDEN,
        "the handler's own check_auth refused a credential the layer accepted"
    );
}

#[tokio::test]
async fn a_session_cookie_authenticates_an_unguarded_route_too() {
    let state = create_test_app_state(Some("test-key".to_string())).await;
    let token = state.browser_sessions.mint_default("operator").await;
    let app = create_test_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/unguarded")
                .header(header::COOKIE, format!("cloto_session={token}"))
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("send request");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_cookie_that_names_no_session_is_refused() {
    let state = create_test_app_state(Some("test-key".to_string())).await;
    // A live session exists; the one presented is simply not it.
    state.browser_sessions.mint_default("operator").await;
    let app = create_test_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/unguarded")
                .header(header::COOKIE, "cloto_session=not-a-session")
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("send request");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn an_expired_session_is_refused() {
    let state = create_test_app_state(Some("test-key".to_string())).await;
    let token = state
        .browser_sessions
        .mint("operator", chrono::Duration::seconds(-1))
        .await;
    let app = create_test_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/unguarded")
                .header(header::COOKIE, format!("cloto_session={token}"))
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("send request");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_revoked_session_is_refused() {
    let state = create_test_app_state(Some("test-key".to_string())).await;
    let token = state.browser_sessions.mint_default("operator").await;
    assert!(state.browser_sessions.revoke(&token).await);
    let app = create_test_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/unguarded")
                .header(header::COOKIE, format!("cloto_session={token}"))
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("send request");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

/// A browser sends every cookie it holds for the origin, so the session is
/// normally one of several. Finding it among neighbours is not incidental.
#[tokio::test]
async fn the_session_is_found_among_the_browsers_other_cookies() {
    let state = create_test_app_state(Some("test-key".to_string())).await;
    let token = state.browser_sessions.mint_default("operator").await;
    let app = create_test_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/unguarded")
                .header(
                    header::COOKIE,
                    format!("cloto-language=ja; cloto_session={token}; theme=dark"),
                )
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("send request");
    assert_eq!(response.status(), StatusCode::OK);
}

/// An unresolvable cookie must not become a refusal of its own. The session
/// branch is one credential among several, so a request that also carries a
/// working one has to keep working — otherwise a stale cookie left in a browser
/// would lock out the key that is still valid.
#[tokio::test]
async fn an_unresolvable_cookie_falls_through_to_the_other_credentials() {
    let state = create_test_app_state(Some("test-key".to_string())).await;
    let app = create_test_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/unguarded?token=test-key")
                .header(header::COOKIE, "cloto_session=stale")
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("send request");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "an unresolvable cookie must fall through to the other credentials, not refuse outright"
    );
}

/// Rotating the admin key must end every session standing on it.
///
/// This is asserted because the version of it that lived as a statement next to
/// the swap was measured to survive its deletion: the whole suite stayed green
/// with rotation no longer revoking anything. The invariant now lives inside
/// `install_admin_api_key`, and this is what says so.
#[tokio::test]
async fn installing_a_new_admin_key_ends_every_browser_session() {
    let state = create_test_app_state(Some("old-key".to_string())).await;
    let token = state.browser_sessions.mint_default("operator").await;
    assert!(state.browser_sessions.resolve(&token).await.is_some());

    assert!(state.install_admin_api_key("new-key".to_string()).await);

    assert!(
        state.browser_sessions.resolve(&token).await.is_none(),
        "a session authorised by the retired key outlived it"
    );
    assert_eq!(state.browser_sessions.live_count().await, 0);
}

/// The other half of the same function: the key really did become live, so the
/// test above cannot pass by rotation having done nothing at all.
#[tokio::test]
async fn installing_a_new_admin_key_makes_it_the_live_one() {
    let state = create_test_app_state(Some("old-key".to_string())).await;
    assert!(state.install_admin_api_key("new-key".to_string()).await);
    assert_eq!(
        state.admin_api_key.read().expect("lock").clone(),
        Some("new-key".to_string())
    );
}
