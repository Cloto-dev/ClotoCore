//! Database migration and initialization tests.
//! Tests that DB init is idempotent and creates required tables.

use sqlx::SqlitePool;

async fn fresh_pool() -> SqlitePool {
    SqlitePool::connect("sqlite::memory:").await.unwrap()
}

#[tokio::test]
async fn test_db_init_is_idempotent() {
    let pool = fresh_pool().await;

    // Running init_db twice should not fail (idempotent migrations)
    cloto_core::db::init_db(&pool, "sqlite::memory:", None)
        .await
        .unwrap();
    cloto_core::db::init_db(&pool, "sqlite::memory:", None)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_migration_creates_required_tables() {
    let pool = fresh_pool().await;
    cloto_core::db::init_db(&pool, "sqlite::memory:", None)
        .await
        .unwrap();

    let tables: Vec<(String,)> =
        sqlx::query_as("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .fetch_all(&pool)
            .await
            .unwrap();

    let table_names: Vec<String> = tables.into_iter().map(|(n,)| n).collect();

    // Core tables required for operation
    for required in &[
        "agents",
        "plugin_settings",
        "plugin_configs",
        "plugin_data",
        "audit_logs",
    ] {
        assert!(
            table_names.contains(&(*required).to_string()),
            "Required table '{}' not found; existing tables: {:?}",
            required,
            table_names
        );
    }
}

#[tokio::test]
async fn test_plugin_data_store_basic_roundtrip() {
    use cloto_shared::PluginDataStore;
    use std::sync::Arc;

    let pool = fresh_pool().await;
    cloto_core::db::init_db(&pool, "sqlite::memory:", None)
        .await
        .unwrap();

    let store = Arc::new(cloto_core::db::SqliteDataStore::new(pool));
    let value = serde_json::json!({"hello": "world", "n": 42});

    store
        .set_json("test.plugin", "my_key", value.clone())
        .await
        .unwrap();
    let retrieved = store.get_json("test.plugin", "my_key").await.unwrap();
    assert_eq!(retrieved, Some(value));
}

#[tokio::test]
async fn test_plugin_data_store_missing_key_returns_none() {
    use cloto_shared::PluginDataStore;
    use std::sync::Arc;

    let pool = fresh_pool().await;
    cloto_core::db::init_db(&pool, "sqlite::memory:", None)
        .await
        .unwrap();

    let store = Arc::new(cloto_core::db::SqliteDataStore::new(pool));
    let result = store
        .get_json("test.plugin", "nonexistent_key")
        .await
        .unwrap();
    assert!(result.is_none());
}

/// Build a minimal McpServerRecord for the trust_level persistence tests.
/// `name` is caller-supplied so tests can run in parallel without collisions.
fn make_record(name: &str, trust_level: Option<&str>) -> cloto_core::db::McpServerRecord {
    cloto_core::db::McpServerRecord {
        name: name.to_string(),
        command: "/bin/true".to_string(),
        trust_level: trust_level.map(str::to_string),
        created_at: 1,
        is_active: true,
        ..Default::default()
    }
}

/// The new column is nullable, so existing rows (pre-migration) load back
/// as `trust_level: None` and the downstream isolation fallback keeps them
/// on Standard. Regression guard for `20260415000000_add_mcp_server_trust_level.sql`.
#[tokio::test]
async fn test_trust_level_column_is_nullable_and_defaults_null() {
    let pool = fresh_pool().await;
    cloto_core::db::init_db(&pool, "sqlite::memory:", None)
        .await
        .unwrap();

    let record = make_record("test.null_trust", None);
    cloto_core::db::save_mcp_server(&pool, &record)
        .await
        .unwrap();

    let loaded = cloto_core::db::load_active_mcp_servers(&pool)
        .await
        .unwrap();
    let row = loaded.iter().find(|r| r.name == "test.null_trust").unwrap();
    assert_eq!(row.trust_level, None);
}

/// A `Some("core")` record survives the round-trip. This is the path the
/// isolation resolver at `mcp.rs:867` relies on — without it, config.mgp
/// is always `None` and isolation falls back to Standard regardless of
/// what the registry / handshake declares.
#[tokio::test]
async fn test_trust_level_roundtrips_through_save_and_load() {
    let pool = fresh_pool().await;
    cloto_core::db::init_db(&pool, "sqlite::memory:", None)
        .await
        .unwrap();

    let record = make_record("test.core_trust", Some("core"));
    cloto_core::db::save_mcp_server(&pool, &record)
        .await
        .unwrap();

    let loaded = cloto_core::db::load_active_mcp_servers(&pool)
        .await
        .unwrap();
    let row = loaded.iter().find(|r| r.name == "test.core_trust").unwrap();
    assert_eq!(row.trust_level.as_deref(), Some("core"));
}

/// Partial writes (e.g. lifecycle re-save that doesn't know trust_level)
/// must not clobber a previously-stored value. The ON CONFLICT clause in
/// `save_mcp_server` uses `COALESCE(excluded.trust_level, mcp_servers.trust_level)`
/// to preserve the existing row's value when the new record passes `None`.
#[tokio::test]
async fn test_trust_level_upsert_preserves_prior_value_on_partial_write() {
    let pool = fresh_pool().await;
    cloto_core::db::init_db(&pool, "sqlite::memory:", None)
        .await
        .unwrap();

    let first = make_record("test.preserve", Some("core"));
    cloto_core::db::save_mcp_server(&pool, &first)
        .await
        .unwrap();

    let second = make_record("test.preserve", None);
    cloto_core::db::save_mcp_server(&pool, &second)
        .await
        .unwrap();

    let loaded = cloto_core::db::load_active_mcp_servers(&pool)
        .await
        .unwrap();
    let row = loaded.iter().find(|r| r.name == "test.preserve").unwrap();
    assert_eq!(
        row.trust_level.as_deref(),
        Some("core"),
        "UPSERT must not clobber trust_level with NULL"
    );
}

/// `set_marketplace_fields` is the primary write path after a marketplace
/// install. Ensure it persists trust_level alongside version/marketplace_id.
#[tokio::test]
async fn test_set_marketplace_fields_persists_trust_level() {
    let pool = fresh_pool().await;
    cloto_core::db::init_db(&pool, "sqlite::memory:", None)
        .await
        .unwrap();

    let record = make_record("test.marketplace", None);
    cloto_core::db::save_mcp_server(&pool, &record)
        .await
        .unwrap();

    cloto_core::db::set_marketplace_fields(
        &pool,
        "test.marketplace",
        "1.2.3",
        "test.marketplace",
        Some("core"),
    )
    .await
    .unwrap();

    let loaded = cloto_core::db::load_active_mcp_servers(&pool)
        .await
        .unwrap();
    let row = loaded
        .iter()
        .find(|r| r.name == "test.marketplace")
        .unwrap();
    assert_eq!(row.trust_level.as_deref(), Some("core"));
    assert_eq!(row.installed_version.as_deref(), Some("1.2.3"));
    // marketplace_id must persist so it can flow into McpServerConfig and
    // McpServerInfo.marketplace_id, gating the dashboard's MGP purple card
    // for catalog-originated servers (lib/mgp.ts isMgpServer).
    assert_eq!(row.marketplace_id.as_deref(), Some("test.marketplace"));
}

/// Manually-registered (non-marketplace) servers must persist `marketplace_id`
/// as NULL so the dashboard distinguishes them from catalog-origin servers
/// in `lib/mgp.ts isMgpServer()` / `isVerified()`.
#[tokio::test]
async fn test_manual_register_leaves_marketplace_id_null() {
    let pool = fresh_pool().await;
    cloto_core::db::init_db(&pool, "sqlite::memory:", None)
        .await
        .unwrap();

    let record = make_record("test.manual", None);
    cloto_core::db::save_mcp_server(&pool, &record)
        .await
        .unwrap();

    let loaded = cloto_core::db::load_active_mcp_servers(&pool)
        .await
        .unwrap();
    let row = loaded.iter().find(|r| r.name == "test.manual").unwrap();
    assert!(
        row.marketplace_id.is_none(),
        "manual register must leave marketplace_id NULL"
    );
}

/// 0.6.6 Phase C+D smoke: assert the in-tree migrations rewrote the
/// `memory.cpersona` seed (from `20260309100000_heal_default_agent.sql`) to
/// the catalog-canonical `cpersona`, back-filled
/// `agent.cloto_default.preferred_memory`, and renamed the FK-bound
/// `mcp_access_control` row to match. The heal-migration seed gives us a
/// realistic pre-0.6.6 starting state without any manual setup.
#[tokio::test]
async fn test_phase_cd_migrations_rewrite_legacy_memory_cpersona() {
    let pool = fresh_pool().await;
    cloto_core::db::init_db(&pool, "sqlite::memory:", None)
        .await
        .unwrap();

    // Phase D: legacy `memory.cpersona` row in mcp_servers (seeded by
    // 20260309100000_heal_default_agent.sql) should have been renamed.
    let legacy_server_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM mcp_servers WHERE name = 'memory.cpersona'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        legacy_server_count, 0,
        "Phase D should rename 'memory.cpersona' out of mcp_servers"
    );

    let new_server_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM mcp_servers WHERE name = 'cpersona'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        new_server_count, 1,
        "Phase D should leave exactly one 'cpersona' row in mcp_servers"
    );

    // Phase C: agent.cloto_default.preferred_memory back-filled to 'cpersona'.
    let preferred: Option<String> = sqlx::query_scalar(
        "SELECT json_extract(metadata, '$.preferred_memory') \
         FROM agents WHERE id = 'agent.cloto_default'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        preferred.as_deref(),
        Some("cpersona"),
        "Phase C should back-fill preferred_memory='cpersona'"
    );

    // Phase D step 3: the migration-issued server_grant for the default agent
    // points at the new id.
    let new_grant_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM mcp_access_control \
         WHERE server_id = 'cpersona' AND agent_id = 'agent.cloto_default' \
           AND entry_type = 'server_grant'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        new_grant_count >= 1,
        "Phase D should leave a 'cpersona' server_grant for the default agent"
    );

    // Phase D leaves no legacy access_control rows.
    let legacy_grant_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM mcp_access_control WHERE server_id = 'memory.cpersona'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        legacy_grant_count, 0,
        "Phase D should DELETE legacy access_control rows"
    );
}

/// 0.6.6 Phase C+D idempotency: replaying Phase C against an init_db'd schema
/// is a no-op — the EXISTS gate and the empty-or-null `preferred_memory`
/// guard both fail, so a user who already picked a memory plugin manually
/// keeps their choice.
#[tokio::test]
async fn test_phase_c_migration_preserves_manual_choice() {
    let pool = fresh_pool().await;
    cloto_core::db::init_db(&pool, "sqlite::memory:", None)
        .await
        .unwrap();

    // Overwrite the back-filled preferred_memory with a manual choice
    // (simulating a user who picked a custom memory plugin via dashboard).
    sqlx::query(
        "UPDATE agents \
         SET metadata = json_set(COALESCE(metadata, '{}'), '$.preferred_memory', 'custom.memory') \
         WHERE id = 'agent.cloto_default'",
    )
    .execute(&pool)
    .await
    .unwrap();

    // Re-execute Phase C SQL: the guard should preserve the manual choice.
    let phase_c = include_str!("../migrations/20260524000000_backfill_default_memory.sql");
    sqlx::raw_sql(phase_c).execute(&pool).await.unwrap();

    let preferred: Option<String> = sqlx::query_scalar(
        "SELECT json_extract(metadata, '$.preferred_memory') \
         FROM agents WHERE id = 'agent.cloto_default'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        preferred.as_deref(),
        Some("custom.memory"),
        "Phase C must not overwrite a non-empty manual preferred_memory"
    );
}
