//! Phase 5 prep: integration tests for [`fetch_registry`].
//!
//! Essential 3 cases from the Phase 5 catalog cutover design memo:
//! 1. happy path: 200 OK + valid registry → `Fresh` + cache populated
//! 2. stale fallback: 1st 200 → 2nd 503 → `Stale { cached, error }`
//! 3. forward-compat shape: registry with unknown fields → parse succeeds

use cloto_core::handlers::marketplace::{fetch_registry, FetchResult};
use cloto_core::test_utils::create_test_app_state;
use tokio::sync::Mutex;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// `CLOTO_CATALOG_URL` is process-global; tests that mutate it run
/// serially via this lock so concurrent test threads do not race.
/// Async-aware so it can be held across `.await`.
static ENV_LOCK: Mutex<()> = Mutex::const_new(());

const VALID_REGISTRY_BODY: &str = r#"{
    "schema_version": 1,
    "updated_at": "2026-05-08T00:00:00Z",
    "servers": [
        {
            "id": "demo-server",
            "name": "Demo",
            "description": "Demo server",
            "category": "test",
            "version": "0.1.0",
            "directory": "demo",
            "dependencies": [],
            "env_vars": [],
            "tags": [],
            "trust_level": "standard",
            "auto_restart": false,
            "icon": null,
            "runtime": "python",
            "seal": "sha256:0000000000000000000000000000000000000000000000000000000000000000"
        }
    ]
}"#;

#[tokio::test]
async fn fetch_registry_happy_path() {
    let _guard = ENV_LOCK.lock().await;
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/registry.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(VALID_REGISTRY_BODY))
        .expect(1)
        .mount(&mock)
        .await;

    set_catalog_url(&format!("{}/registry.json", mock.uri()));
    let state = create_test_app_state(None).await;

    let result = fetch_registry(&state, true).await.expect("fresh fetch");

    assert!(
        matches!(result, FetchResult::Fresh(_)),
        "expected Fresh, got {result:?}"
    );
    assert!(!result.is_stale());
    assert!(result.stale_reason().is_none());
    assert_eq!(result.registry().servers.len(), 1);
    assert_eq!(result.registry().servers[0].id, "demo-server");

    let cache = state.marketplace_cache.read().await;
    assert!(cache.data.is_some(), "cache should be populated");
    assert!(cache.fetched_at.is_some(), "cache fetched_at should be set");
}

#[tokio::test]
async fn fetch_registry_stale_fallback_on_upstream_failure() {
    let _guard = ENV_LOCK.lock().await;
    let mock = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/registry.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(VALID_REGISTRY_BODY))
        .up_to_n_times(1)
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/registry.json"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&mock)
        .await;

    set_catalog_url(&format!("{}/registry.json", mock.uri()));
    let state = create_test_app_state(None).await;

    let first = fetch_registry(&state, true).await.expect("first fetch");
    assert!(
        matches!(first, FetchResult::Fresh(_)),
        "first call should be Fresh, got {first:?}"
    );

    let second = fetch_registry(&state, true)
        .await
        .expect("second fetch should fall back to cache, not error");
    match &second {
        FetchResult::Stale { cached, error } => {
            assert_eq!(cached.servers.len(), 1, "cached registry preserved");
            assert!(
                error.contains("503"),
                "stale error should mention 503: {error}"
            );
        }
        FetchResult::Fresh(_) => panic!("expected Stale on second call, got Fresh"),
    }
    assert!(second.is_stale());
    assert!(second.stale_reason().is_some());
}

#[tokio::test]
async fn fetch_registry_forward_compat_unknown_fields() {
    let _guard = ENV_LOCK.lock().await;
    let mock = MockServer::start().await;

    let body_with_extras = r#"{
        "schema_version": 1,
        "updated_at": "2026-05-08T00:00:00Z",
        "future_top_level_field": "ignored",
        "servers": [
            {
                "id": "demo-server",
                "name": "Demo",
                "description": "Demo server",
                "category": "test",
                "version": "0.1.0",
                "directory": "demo",
                "dependencies": [],
                "env_vars": [],
                "tags": [],
                "trust_level": "standard",
                "auto_restart": false,
                "icon": null,
                "runtime": "python",
                "seal": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
                "future_per_server_field": {"nested": [1, 2, 3]}
            }
        ]
    }"#;
    Mock::given(method("GET"))
        .and(path("/registry.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body_with_extras))
        .expect(1)
        .mount(&mock)
        .await;

    set_catalog_url(&format!("{}/registry.json", mock.uri()));
    let state = create_test_app_state(None).await;

    let result = fetch_registry(&state, true)
        .await
        .expect("forward-compat fetch");
    assert!(
        matches!(result, FetchResult::Fresh(_)),
        "expected Fresh on forward-compat shape, got {result:?}"
    );
    assert_eq!(result.registry().servers.len(), 1);
    assert_eq!(result.registry().servers[0].id, "demo-server");
}

fn set_catalog_url(url: &str) {
    std::env::set_var("CLOTO_CATALOG_URL", url);
}
