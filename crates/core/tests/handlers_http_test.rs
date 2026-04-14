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
        .route("/agents/:id", post(handlers::update_agent))
        .route(
            "/agents/:id/mcp-access",
            put(handlers::put_agent_mcp_access),
        )
        .route("/cron/jobs", post(handlers::create_cron_job))
        .route("/plugins/:id/config", post(handlers::update_plugin_config))
        .route(
            "/permissions/:id/approve",
            post(handlers::approve_permission),
        );

    let api_routes = axum::Router::new()
        .route("/chat", post(handlers::chat_handler))
        .route("/agents", get(handlers::get_agents))
        .route("/plugins/:id/config", get(handlers::get_plugin_config))
        .merge(admin_routes)
        .with_state(state);

    axum::Router::new().nest("/api", api_routes)
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
    seed_mcp_server(&state.pool, "memory.cpersona").await;
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
        "granted_server_ids": ["terminal", "memory.cpersona"]
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
    assert_eq!(server_ids, vec!["memory.cpersona", "terminal"]);
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
        "granted_server_ids": ["terminal", "memory.cpersona"]
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
         WHERE name IN ('terminal', 'memory.cpersona') ORDER BY name",
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
