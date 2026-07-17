//! Marketplace endpoints for discovering and installing MCP servers
//! from the ClotoHub catalog (legacy: the clotohub-servers registry,
//! formerly named cloto-mcp-servers).

use axum::{
    extract::{ConnectInfo, Path, Query, State},
    http::HeaderMap,
    response::sse::{Event, Sse},
    Json,
};
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, convert::Infallible, path::PathBuf, sync::Arc, time::Duration};
use tracing::{error, info, warn};

use super::setup::{emit, SetupProgressEvent};
use crate::{
    AppError, AppResult, AppState, CHILD_PROCESS_TIMEOUT_SECS, TARBALL_DOWNLOAD_TIMEOUT_SECS,
};

/// Timeout for fetching the marketplace registry JSON from GitHub. Small
/// payload, so anything past this usually means a network stall.
const MARKETPLACE_REGISTRY_FETCH_TIMEOUT_SECS: u64 = 30;

// ── Registry types ──────────────────────────────────────────────────

/// Per-entry registry shape, sourced from `mgp-sdk` v0.2.0 so ClotoCore and
/// the ClotoHub.dev catalog feed share one wire definition. The `install`
/// field is `Option<InstallShape>` — `None` falls back to ClotoCore's
/// legacy monorepo-tarball install path; `Some(_)` will branch on
/// `install.source` once `run_install` is updated.
pub use mgp_sdk::shape::{InstallShape, RegistryEntry};
pub use mgp_sdk::types::EnvVarDef;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Registry {
    pub schema_version: u32,
    pub updated_at: String,
    pub servers: Vec<RegistryEntry>,
    /// Hub-served setup-preset curation (ONBOARDING_MODERNIZATION_DESIGN.md
    /// §3). Absent on legacy registries — `#[serde(default)]` keeps old
    /// catalogs parseable; the dashboard falls back to its bundled presets
    /// when the list is empty.
    #[serde(default)]
    pub collections: Vec<CollectionEntry>,
}

/// One hub-curated setup preset, passed through to the dashboard verbatim.
/// Shape mirrors the hub's `catalog::collections::Collection`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionEntry {
    pub id: String,
    /// Lucide icon name hint (e.g. "layers"); unknown names fall back
    /// client-side.
    #[serde(default)]
    pub icon: String,
    pub servers: Vec<String>,
    #[serde(default)]
    pub default_engine: Option<String>,
}

// ── Catalog cache ───────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct CatalogCache {
    pub data: Option<Registry>,
    pub fetched_at: Option<tokio::time::Instant>,
}

const CACHE_TTL: Duration = Duration::from_hours(1); // 1 hour

/// Default upstream catalog URL. Phase 5d-2 flipped this from the
/// legacy `raw.githubusercontent.com/Cloto-dev/cloto-mcp-servers/...`
/// path to the curated ClotoHub.dev catalog endpoint. Overridable via
/// `CLOTO_CATALOG_URL` (e.g. to point a dev build at a local server,
/// or to pin a release to the legacy GitHub-served registry).
const DEFAULT_CATALOG_URL: &str = "https://hub.cloto.dev/api/catalog";

/// Default tarball URL template for the *legacy* monorepo install path
/// (`install.source.kind == "monorepo_directory"` entries). `{ref}` is
/// replaced with the git ref (currently always `master`). Catalog
/// entries served by ClotoHub.dev dispatch through `run_install`
/// (`install.source: Git/RawUrl/Pypi/Docker`) and never hit this
/// template — it is preserved only so a `CLOTO_CATALOG_URL` override
/// pointing back at the legacy registry still resolves correctly.
/// Overridable via `CLOTO_TARBALL_URL_TEMPLATE`. Phase 4 renamed the
/// monorepo `cloto-mcp-servers` → `clotohub-servers` and made it
/// private, so the default below only works with authenticated access;
/// unauthenticated dev overrides must point `CLOTO_TARBALL_URL_TEMPLATE`
/// at a reachable mirror.
const DEFAULT_TARBALL_URL_TEMPLATE: &str =
    "https://api.github.com/repos/Cloto-dev/clotohub-servers/tarball/{ref}";

/// Resolve the marketplace catalog URL, honoring `CLOTO_CATALOG_URL` if set.
fn catalog_url() -> String {
    std::env::var("CLOTO_CATALOG_URL").unwrap_or_else(|_| DEFAULT_CATALOG_URL.to_string())
}

// ── Hub seal JWKS cache (bug-394 proper fix) ────────────────────────

/// In-memory cache of the hub's Ed25519 signing keys, fetched from its
/// JWKS endpoint (`/api/seal/keys`) and keyed by `kid`. Used at install
/// time to verify a catalog entry's `signature_payload.ed25519` offline.
///
/// In-memory only (re-fetched after restart): the JWKS is public and
/// cheap to retrieve, and a persistent copy would lag key rotation.
#[derive(Debug, Default)]
pub struct JwksCache {
    pub keys: HashMap<String, mgp_seal::ed25519::PublicKey>,
    pub fetched_at: Option<tokio::time::Instant>,
}

const JWKS_CACHE_TTL: Duration = Duration::from_hours(1);

/// Timeout for fetching the hub JWKS. Tiny payload; a stall past this
/// means the network is down and the caller takes the benign
/// `jwks_unavailable` path (register unsealed → untrusted at spawn).
const SEAL_JWKS_FETCH_TIMEOUT_SECS: u64 = 15;

/// Resolve the hub JWKS URL. `CLOTO_SEAL_JWKS_URL` overrides explicitly
/// (e.g. a dev hub); otherwise it is derived from the catalog URL by
/// swapping the `/api/catalog` suffix for `/api/seal/keys`. A catalog
/// override that is not hub-shaped (e.g. the legacy GitHub-served
/// registry) yields `None` — there is no JWKS to fetch, and entries
/// verify down the unsigned path.
fn seal_jwks_url() -> Option<String> {
    if let Ok(v) = std::env::var("CLOTO_SEAL_JWKS_URL") {
        let v = v.trim().to_string();
        return (!v.is_empty()).then_some(v);
    }
    catalog_url()
        .strip_suffix("/api/catalog")
        .map(|base| format!("{base}/api/seal/keys"))
}

/// Fetch and parse the hub JWKS into a `kid → PublicKey` map. Retired
/// (revoked-annotated) keys are kept: a seal signed before a rotation
/// still verifies under the key that signed it, and revocation policy
/// is a separate concern from signature math. Individual keys that
/// fail to parse are skipped, not fatal.
async fn fetch_jwks(url: &str) -> anyhow::Result<HashMap<String, mgp_seal::ed25519::PublicKey>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(SEAL_JWKS_FETCH_TIMEOUT_SECS))
        .build()?;
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("JWKS fetch failed: HTTP {}", resp.status());
    }
    let body: serde_json::Value = resp.json().await?;
    let mut keys = HashMap::new();
    for jwk in body
        .get("keys")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        match mgp_seal::ed25519::public_key_from_jwk(jwk) {
            Ok((pk, kid)) => {
                keys.insert(kid.as_str().to_string(), pk);
            }
            Err(e) => warn!("Skipping unparseable JWK in hub JWKS: {e}"),
        }
    }
    Ok(keys)
}

/// Resolve the hub signing key for `kid`, fetching / re-fetching the
/// JWKS as needed. Returns `None` when the JWKS is unreachable, not
/// configured, or does not contain `kid` even after a forced re-fetch
/// (rotation lag) — callers treat that as the benign `jwks_unavailable`
/// outcome, never as tampering.
async fn hub_signing_key(state: &AppState, kid: &str) -> Option<mgp_seal::ed25519::PublicKey> {
    // Fast path: fresh cache that already knows this kid.
    {
        let cache = state.seal_jwks_cache.read().await;
        if let Some(at) = cache.fetched_at {
            if at.elapsed() < JWKS_CACHE_TTL {
                if let Some(pk) = cache.keys.get(kid) {
                    return Some(*pk);
                }
                // Fresh but kid-miss: fall through to one forced re-fetch
                // so a just-rotated key is picked up without waiting out
                // the TTL.
            }
        }
    }

    let url = seal_jwks_url()?;
    match fetch_jwks(&url).await {
        Ok(keys) => {
            let mut cache = state.seal_jwks_cache.write().await;
            cache.keys = keys;
            cache.fetched_at = Some(tokio::time::Instant::now());
            cache.keys.get(kid).copied()
        }
        Err(e) => {
            warn!(url = %url, "Hub JWKS unreachable: {e}");
            // Stale cache beats nothing: an old key set can still verify
            // seals signed before the outage.
            let cache = state.seal_jwks_cache.read().await;
            cache.keys.get(kid).copied()
        }
    }
}

/// Extract the `kid` of the Ed25519 signature riding on a catalog entry,
/// if any. Used to resolve the signing key before the (sync) seal
/// decision in [`local_seal_for_install`].
fn entry_signature_kid(entry: &RegistryEntry) -> Option<String> {
    entry
        .signature_payload
        .as_ref()?
        .get("ed25519")?
        .get("key_id")?
        .as_str()
        .map(str::to_owned)
}

/// Resolve the tarball download URL for a given git ref, honoring
/// `CLOTO_TARBALL_URL_TEMPLATE` if set. The template must contain `{ref}`,
/// which is replaced with `ref_name`.
fn tarball_url(ref_name: &str) -> String {
    let template = std::env::var("CLOTO_TARBALL_URL_TEMPLATE")
        .unwrap_or_else(|_| DEFAULT_TARBALL_URL_TEMPLATE.to_string());
    template.replace("{ref}", ref_name)
}

// ── Registry fetch result ───────────────────────────────────────────

/// Outcome of [`fetch_registry`]. `Stale` means the upstream returned a
/// non-success status (or, by extension, refused to serve), but the
/// in-process cache had a previously-fetched copy that callers can still
/// use. Surfaces upstream errors to the UI instead of swallowing them.
#[derive(Debug, Clone)]
pub enum FetchResult {
    Fresh(Registry),
    Stale { cached: Registry, error: String },
}

impl FetchResult {
    /// Discard freshness info and return the underlying [`Registry`].
    #[must_use]
    pub fn into_registry(self) -> Registry {
        match self {
            Self::Fresh(r) | Self::Stale { cached: r, .. } => r,
        }
    }

    /// Borrow the underlying [`Registry`].
    #[must_use]
    pub fn registry(&self) -> &Registry {
        match self {
            Self::Fresh(r) | Self::Stale { cached: r, .. } => r,
        }
    }

    /// `true` when the data came from a cached fallback after an upstream
    /// failure.
    #[must_use]
    pub fn is_stale(&self) -> bool {
        matches!(self, Self::Stale { .. })
    }

    /// Reason for the stale fallback, if any.
    #[must_use]
    pub fn stale_reason(&self) -> Option<&str> {
        match self {
            Self::Stale { error, .. } => Some(error.as_str()),
            Self::Fresh(_) => None,
        }
    }
}

// ── Response types ──────────────────────────────────────────────────

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Serialize)]
pub struct CatalogEntry {
    // Registry fields
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub version: String,
    pub directory: String,
    pub dependencies: Vec<String>,
    pub env_vars: Vec<EnvVarDef>,
    pub optional_env_vars: Vec<EnvVarDef>,
    pub tags: Vec<String>,
    pub trust_level: String,
    pub auto_restart: bool,
    pub icon: Option<String>,
    pub runtime: String,
    pub changelog: Option<String>,
    /// MGP Magic Seal in `sha256:HEX` form (registry-side, before install).
    /// `None` means the upstream registry did not ship a seal — at runtime
    /// the kernel will force trust_level=untrusted (MGP v0.6.3 §10 inv 3).
    /// Surfaced to the dashboard so MarketplaceCard can show a verified /
    /// unverified badge before the user installs.
    pub seal: Option<String>,
    // Merged local state
    pub installed: bool,
    pub installed_version: Option<String>,
    pub update_available: bool,
    pub running: bool,
}

#[derive(Debug, Deserialize)]
pub struct CatalogQuery {
    #[serde(default)]
    pub force_refresh: bool,
}

#[derive(Debug, Deserialize)]
pub struct InstallRequest {
    pub server_id: String,
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
    #[serde(default)]
    pub auto_start: Option<bool>,
    /// In-place update of an already-installed server (the marketplace "Update"
    /// button). When false/absent, an already-installed server is rejected
    /// (fresh-install semantics, unchanged). When true, the server is stopped,
    /// re-vendored, and reconnected in place, preserving its DB row, grants, and
    /// configured env. See [`reject_existing`].
    #[serde(default)]
    pub update: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct BatchInstallRequest {
    pub server_ids: Vec<String>,
    #[serde(default)]
    pub auto_start: Option<bool>,
}

// ── Endpoints ───────────────────────────────────────────────────────

/// GET /api/marketplace/catalog — fetch registry and merge with local state.
pub async fn catalog_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<CatalogQuery>,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;

    let fetched = fetch_registry(&state, query.force_refresh)
        .await
        .map_err(|e| {
            AppError::Internal(anyhow::anyhow!("Failed to fetch marketplace catalog: {e}"))
        })?;
    let is_stale = fetched.is_stale();
    let stale_reason = fetched.stale_reason().map(str::to_string);
    let registry = fetched.into_registry();

    let marketplace_servers = crate::db::mcp::get_marketplace_servers(&state.pool)
        .await
        .unwrap_or_default();

    let running_servers = state.mcp_manager.list_servers().await;

    let entries: Vec<CatalogEntry> = registry
        .servers
        .iter()
        .map(|entry| {
            let mp_record = marketplace_servers
                .iter()
                .find(|r| r.marketplace_id.as_deref() == Some(&entry.id));
            // Installed if: DB marketplace record is active, OR server files
            // exist on disk (covers Config servers installed by batch setup).
            // `effective_install_dir` mirrors the install path's fallback so
            // the on-disk probe targets the same dir the installer wrote to,
            // even when `entry.directory` is empty.
            let db_installed = mp_record.is_some_and(|r| r.is_active);
            let files_installed = state
                .data_dir
                .join("mcp-servers")
                .join(effective_install_dir(entry))
                .is_dir();
            let installed = db_installed || files_installed;
            let installed_version = mp_record.and_then(|r| r.installed_version.clone());
            let update_available = installed_version
                .as_deref()
                .is_some_and(|iv| iv != entry.version);
            let running = running_servers.iter().any(|s| {
                s.id == entry.id
                    && matches!(
                        s.status,
                        crate::managers::mcp_types::ServerStatus::Connected
                    )
            });

            CatalogEntry {
                id: entry.id.clone(),
                name: entry.name.clone(),
                description: entry.description.clone(),
                category: entry.category.clone(),
                version: entry.version.clone(),
                directory: entry.directory.clone(),
                dependencies: entry.dependencies.clone(),
                env_vars: entry.env_vars.clone(),
                optional_env_vars: entry.optional_env_vars.clone(),
                tags: entry.tags.clone(),
                trust_level: entry.trust_level.clone(),
                auto_restart: entry.auto_restart,
                icon: entry.icon.clone(),
                runtime: entry.runtime.clone(),
                changelog: entry.changelog.clone(),
                seal: entry.seal.clone(),
                installed,
                installed_version,
                update_available,
                running,
            }
        })
        .collect();

    let cached_at = {
        let cache = state.marketplace_cache.read().await;
        cache.fetched_at.map_or_else(
            || "just now".to_string(),
            |t| format!("{:.0}s ago", t.elapsed().as_secs()),
        )
    };

    super::ok_data(serde_json::json!({
        "servers": entries,
        // Hub-curated setup presets, passed through for the dashboard's
        // preset resolution chain (bundled presets are its offline floor).
        "collections": registry.collections,
        "cached_at": cached_at,
        "is_stale": is_stale,
        "stale_reason": stale_reason,
    }))
}

/// Whether the install endpoint should reject an already-present server.
///
/// A fresh install rejects an existing server; an update (the marketplace
/// "Update" button) re-vendors it in place, so the reject is suppressed. An
/// update of a not-yet-installed server falls through to a normal install
/// (idempotent — safe if the row was removed out-of-band).
fn reject_existing(exists: bool, is_update: bool) -> bool {
    exists && !is_update
}

/// The currently-stored env map for an installed server (empty if absent or
/// unparseable). Used by the update path to preserve user-entered secrets that
/// the install dialog re-sends as empty registry defaults.
async fn existing_env(pool: &sqlx::SqlitePool, server_id: &str) -> HashMap<String, String> {
    crate::db::mcp::load_active_mcp_servers(pool)
        .await
        .ok()
        .and_then(|servers| {
            servers
                .into_iter()
                .find(|s| s.name == server_id)
                .and_then(|s| serde_json::from_str(&s.env).ok())
        })
        .unwrap_or_default()
}

/// POST /api/marketplace/install — install a server from the marketplace.
pub async fn install_handler(
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<InstallRequest>,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;

    // Tier 1 rate limit: 5 req/min per IP for heavy operations
    if !state.install_limiter.check(addr.ip()) {
        tracing::warn!(ip = %addr.ip(), "Install rate limit exceeded");
        return Err(AppError::Validation(
            "Too many install requests. Please wait before trying again.".to_string(),
        ));
    }

    // Validate server_id against cached registry
    let registry = {
        let cache = state.marketplace_cache.read().await;
        cache.data.clone()
    };
    let registry = match registry {
        Some(r) => r,
        None => fetch_registry(&state, false)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("Registry not available: {e}")))?
            .into_registry(),
    };

    let entry = registry
        .servers
        .iter()
        .find(|s| s.id == request.server_id)
        .cloned()
        .ok_or_else(|| {
            AppError::Validation(format!(
                "Server '{}' not found in registry",
                request.server_id
            ))
        })?;

    // An already-installed server is rejected only for a fresh install; an
    // update (the marketplace "Update" button) re-vendors in place instead.
    let exists = crate::db::mcp::server_exists_in_db(&state.pool, &request.server_id)
        .await
        .unwrap_or(false);
    let is_update = request.update.unwrap_or(false);
    if reject_existing(exists, is_update) {
        return Err(AppError::Validation(format!(
            "Server '{}' is already installed",
            request.server_id
        )));
    }

    // Prevent concurrent installs (shared with bootstrap)
    let was_running = state
        .setup_in_progress
        .swap(true, std::sync::atomic::Ordering::SeqCst);
    if was_running {
        return Err(AppError::Validation(
            "Another installation is already in progress".to_string(),
        ));
    }

    let state_clone = state.clone();
    // On update, preserve the server's currently-stored env (e.g. API keys the
    // install dialog re-sends as empty defaults): start from the stored env and
    // overlay only non-empty values from the request. save_mcp_server replaces
    // env wholesale, so without this an update would wipe configured secrets.
    let env_overrides = {
        let request_env = request.env.unwrap_or_default();
        if is_update && exists {
            let mut merged = existing_env(&state.pool, &request.server_id).await;
            for (k, v) in request_env {
                if !v.is_empty() {
                    merged.insert(k, v);
                }
            }
            merged
        } else {
            request_env
        }
    };
    let auto_start = request.auto_start.unwrap_or(true);

    let handle = tokio::spawn(async move {
        // For an update, stop the running instance first so re-vendoring does
        // not race a live child and so connect_server (which refuses an
        // already-Connected handle) can replace the now-Disconnected one. The
        // DB row and grants are untouched (unlike remove_server). A not-running
        // server yields an ignorable error.
        if is_update {
            if let Err(e) = state_clone.mcp_manager.stop_server(&entry.id).await {
                tracing::debug!("update: stop_server({}) before re-vendor: {e}", entry.id);
            }
        }
        let result = run_install(&state_clone, &entry, env_overrides, auto_start).await;
        state_clone
            .setup_in_progress
            .store(false, std::sync::atomic::Ordering::SeqCst);
        if let Err(e) = result {
            error!("Marketplace install failed for {}: {e}", entry.id);
        }
        // Always emit Complete so the frontend SSE listener can close.
        emit(&state_clone.setup_progress_tx, SetupProgressEvent::Complete);
    });
    // bug-366: track the install task so shutdown can abort it and reap any
    // orphaned child uv/pip processes (kill_on_drop) instead of leaking them.
    *state.install_task.lock().await = Some(handle);

    super::ok_data(serde_json::json!({ "started": true, "server_id": request.server_id }))
}

/// GET /api/marketplace/progress — SSE progress stream (reuses setup events).
pub async fn marketplace_progress_handler(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut rx = state.setup_progress_tx.subscribe();

    let stream = async_stream::stream! {
        yield Ok(Event::default().event("handshake").data("connected"));

        loop {
            match rx.recv().await {
                Ok(evt) => {
                    if let Ok(json) = serde_json::to_string(&evt) {
                        yield Ok(Event::default().event("setup").data(json));
                    }
                    if matches!(evt, SetupProgressEvent::Complete) {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("Marketplace SSE stream lagged by {} messages", n);
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }
    };

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

/// DELETE /api/marketplace/servers/:id — uninstall a marketplace server.
pub async fn uninstall_handler(
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    State(state): State<Arc<AppState>>,
    Path(server_id): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;

    // Tier 1 rate limit: 5 req/min per IP for heavy operations
    if !state.install_limiter.check(addr.ip()) {
        tracing::warn!(ip = %addr.ip(), "Uninstall rate limit exceeded");
        return Err(AppError::Validation(
            "Too many requests. Please wait before trying again.".to_string(),
        ));
    }

    // bug-400: `server_id` arrives unvalidated from the request path and is
    // used below as a deletion candidate joined onto `servers_root`. Reject any
    // value that could escape the servers root *before* it reaches the join +
    // remove_dir_all, otherwise `DELETE /api/marketplace/servers/..` resolves
    // `servers_root.join("..")` to the data dir itself and recursively wipes the
    // SQLite DB, access grants, embeddings, cron jobs and audit logs. (The
    // catalog/args-derived candidates are already collapsed to a single path
    // component; this guards the raw `server_id` fallback. A containment check
    // below backstops every candidate.)
    if server_id.contains('/') || server_id.contains('\\') || server_id.contains("..") {
        return Err(AppError::Validation("Invalid server id".to_string()));
    }

    // bug-392: capture the registered args before remove_server deletes the
    // DB row — the entry-point path is the ground truth for where the
    // install lives on disk (the catalog entry may be gone, and install
    // directories don't always match the server id, e.g. legacy registry
    // installs: id `mind.deepseek`, dir `deepseek`).
    let registered_args: Option<String> =
        sqlx::query_scalar("SELECT args FROM mcp_servers WHERE name = ?")
            .bind(&server_id)
            .fetch_optional(&state.pool)
            .await
            .ok()
            .flatten();

    // Disconnect and remove from manager + DB
    if let Err(e) = state.mcp_manager.remove_server(&server_id).await {
        warn!("remove_server warning for {}: {}", server_id, e);
        // Still proceed with file cleanup even if server wasn't in memory
    }

    // Delete server files from disk. Candidates in order: the catalog
    // cache's `effective_install_dir` (mirrors the install path's
    // fallback), the directory derived from the registered entry-point
    // args, then `server_id` as-is — the installers' convention when
    // `entry.directory` is empty. The first candidate that exists on disk
    // is the install; a miss across all of them is logged instead of
    // silently orphaning the files (bug-392).
    let servers_root = state.data_dir.join("mcp-servers");
    let catalog_dir = {
        let cache = state.marketplace_cache.read().await;
        cache.data.as_ref().and_then(|reg| {
            reg.servers
                .iter()
                .find(|s| s.id == server_id)
                .map(|s| effective_install_dir(s).to_string())
        })
    };
    let args_dir = install_dir_from_args(registered_args.as_deref(), &servers_root);
    let mut candidates: Vec<String> = Vec::new();
    for c in [catalog_dir, args_dir, Some(server_id.clone())]
        .into_iter()
        .flatten()
    {
        if !candidates.contains(&c) {
            candidates.push(c);
        }
    }

    // bug-400 defense-in-depth: the catalog `directory` is hub-served (and the
    // deployed catalog can be stale/malicious), so backstop every candidate with
    // a canonicalized containment check — the resolved target must live strictly
    // under `servers_root` and must not be `servers_root` itself.
    let canonical_root = servers_root.canonicalize().ok();
    let mut removed = false;
    for directory in &candidates {
        let server_dir = servers_root.join(directory);
        if !server_dir.is_dir() {
            continue;
        }
        match (canonical_root.as_ref(), server_dir.canonicalize()) {
            (Some(root), Ok(target)) if target.starts_with(root) && target != *root => {}
            _ => {
                warn!(
                    "Refusing to uninstall '{}': candidate '{}' resolves outside {} \
                     (path traversal blocked)",
                    server_id,
                    directory,
                    servers_root.display()
                );
                continue;
            }
        }
        if let Err(e) = tokio::fs::remove_dir_all(&server_dir).await {
            warn!(
                "Failed to remove server directory {}: {}",
                server_dir.display(),
                e
            );
        } else {
            info!(
                "Removed marketplace server directory: {}",
                server_dir.display()
            );
            removed = true;
        }
        break;
    }
    if !removed {
        warn!(
            "Uninstall of '{}' removed no files: no install directory found under {} \
             (probed {:?})",
            server_id,
            servers_root.display(),
            candidates
        );
    }

    info!("Marketplace server uninstalled: {}", server_id);

    super::ok_data(serde_json::json!({
        "server_id": server_id,
        "uninstalled": true,
    }))
}

/// POST /api/marketplace/batch-install — install multiple servers at once.
/// Used by the setup wizard to install preset servers in a single operation.
pub async fn batch_install_handler(
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<BatchInstallRequest>,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;

    if !state.install_limiter.check(addr.ip()) {
        tracing::warn!(ip = %addr.ip(), "Batch install rate limit exceeded");
        return Err(AppError::Validation(
            "Too many install requests. Please wait before trying again.".to_string(),
        ));
    }

    if request.server_ids.is_empty() {
        return Err(AppError::Validation(
            "server_ids must not be empty".to_string(),
        ));
    }

    // Resolve entries from registry
    let registry = fetch_registry(&state, false)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Registry not available: {e}")))?
        .into_registry();

    let mut entries = Vec::new();
    let mut skipped_ids = Vec::new();
    for id in &request.server_ids {
        if let Some(entry) = registry.servers.iter().find(|s| &s.id == id) {
            entries.push(entry.clone());
        } else {
            warn!("Server '{}' not found in registry, skipping", id);
            skipped_ids.push(id.clone());
        }
    }

    // Prevent concurrent installs
    let was_running = state
        .setup_in_progress
        .swap(true, std::sync::atomic::Ordering::SeqCst);
    if was_running {
        return Err(AppError::Validation(
            "Another installation is already in progress".to_string(),
        ));
    }

    let state_clone = state.clone();
    let auto_start = request.auto_start.unwrap_or(true);

    let handle = tokio::spawn(async move {
        // Notify skipped servers not found in registry (bug-381)
        for id in &skipped_ids {
            emit(
                &state_clone.setup_progress_tx,
                SetupProgressEvent::ServerInstall {
                    server_name: id.clone(),
                    status: "skipped".into(),
                },
            );
        }
        let result = run_batch_install(&state_clone, &entries, auto_start).await;
        state_clone
            .setup_in_progress
            .store(false, std::sync::atomic::Ordering::SeqCst);
        if result.is_ok() {
            // Signal health monitor that initial setup is done — auto-restart can begin.
            state_clone
                .setup_done
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        if let Err(e) = result {
            error!("Batch install failed: {e}");
        }
    });
    // bug-366: track the install task so shutdown can abort it and reap any
    // orphaned child uv/pip processes (kill_on_drop) instead of leaking them.
    *state.install_task.lock().await = Some(handle);

    super::ok_data(serde_json::json!({
        "started": true,
        "server_ids": request.server_ids,
    }))
}

// ── Registry fetch with cache ───────────────────────────────────────

pub async fn fetch_registry(state: &AppState, force_refresh: bool) -> anyhow::Result<FetchResult> {
    // Check cache first
    if !force_refresh {
        let cache = state.marketplace_cache.read().await;
        if let (Some(data), Some(fetched_at)) = (&cache.data, cache.fetched_at) {
            if fetched_at.elapsed() < CACHE_TTL {
                return Ok(FetchResult::Fresh(data.clone()));
            }
        }
    }

    let url = catalog_url();
    info!(url = %url, "Fetching marketplace registry");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(MARKETPLACE_REGISTRY_FETCH_TIMEOUT_SECS))
        .build()?;
    let resp = client.get(&url).send().await?;

    if !resp.status().is_success() {
        let status = resp.status();
        // Fall back to cache if available
        let cache = state.marketplace_cache.read().await;
        if let Some(data) = &cache.data {
            warn!(
                status = %status,
                "Catalog upstream returned non-success, using cached registry"
            );
            return Ok(FetchResult::Stale {
                cached: data.clone(),
                error: format!("HTTP {status}"),
            });
        }
        anyhow::bail!("Failed to fetch registry: HTTP {status}");
    }

    let registry: Registry = resp.json().await?;
    info!(
        "Marketplace registry loaded: {} servers",
        registry.servers.len()
    );

    // Update cache
    let mut cache = state.marketplace_cache.write().await;
    cache.data = Some(registry.clone());
    cache.fetched_at = Some(tokio::time::Instant::now());

    Ok(FetchResult::Fresh(registry))
}

// ── Toolchain detection ─────────────────────────────────────────────

/// Detect whether `cargo` is available in PATH.
fn detect_cargo() -> (bool, Option<String>) {
    let result = std::process::Command::new("cargo")
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();
    match result {
        Ok(output) if output.status.success() => {
            let ver = String::from_utf8_lossy(&output.stdout).trim().to_string();
            (true, Some(ver))
        }
        _ => (false, None),
    }
}

/// Build a Rust MCP server with `cargo build --release`, streaming progress via SSE.
async fn cargo_build_server(
    tx: &tokio::sync::broadcast::Sender<SetupProgressEvent>,
    server_path: &std::path::Path,
    server_name: &str,
) -> anyhow::Result<bool> {
    emit(
        tx,
        SetupProgressEvent::StepStart {
            step: "cargo_build".into(),
            description: format!("Building {} (this may take several minutes)", server_name),
        },
    );

    let mut child = tokio::process::Command::new("cargo")
        .args(["build", "--release"])
        .current_dir(server_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    // Stream stderr for build progress, collect error lines for failure reporting
    let stderr_handle = if let Some(stderr) = child.stderr.take() {
        let tx_clone = tx.clone();
        Some(tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            let mut last_error_lines: Vec<String> = Vec::new();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.contains("Compiling") || line.contains("Downloading") {
                    emit(
                        &tx_clone,
                        SetupProgressEvent::StepProgress {
                            step: "cargo_build".into(),
                            progress: -1.0,
                            detail: line,
                        },
                    );
                } else if line.contains("error") {
                    last_error_lines.push(line);
                    // Keep only last 5 error lines
                    if last_error_lines.len() > 5 {
                        last_error_lines.remove(0);
                    }
                }
            }
            last_error_lines
        }))
    } else {
        None
    };

    let status = match tokio::time::timeout(Duration::from_mins(10), child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(e)) => {
            warn!("cargo build process error: {e}");
            emit(
                tx,
                SetupProgressEvent::StepError {
                    step: "cargo_build".into(),
                    error: format!("cargo build process error: {e}"),
                    recoverable: true,
                },
            );
            return Ok(false);
        }
        Err(_) => {
            warn!("cargo build --release timed out after 10 minutes for {server_name}");
            child.kill().await.ok();
            emit(
                tx,
                SetupProgressEvent::StepError {
                    step: "cargo_build".into(),
                    error: "Build timed out after 10 minutes".into(),
                    recoverable: true,
                },
            );
            return Ok(false);
        }
    };
    let error_lines = if let Some(handle) = stderr_handle {
        handle.await.unwrap_or_default()
    } else {
        vec![]
    };

    if !status.success() {
        let detail = if error_lines.is_empty() {
            "cargo build --release failed. Check Rust toolchain and dependencies.".to_string()
        } else {
            error_lines.join("\n")
        };
        emit(
            tx,
            SetupProgressEvent::StepError {
                step: "cargo_build".into(),
                error: detail,
                recoverable: false,
            },
        );
        return Ok(false);
    }

    emit(
        tx,
        SetupProgressEvent::StepComplete {
            step: "cargo_build".into(),
        },
    );
    Ok(true)
}

/// Resolve the binary path for a Rust MCP server after `cargo build --release`.
fn rust_binary_path(server_path: &std::path::Path, entry: &RegistryEntry) -> PathBuf {
    let bin_name = entry
        .bin_name
        .clone()
        .unwrap_or_else(|| format!("mgp-{}", entry.directory));
    let bin_filename = if cfg!(windows) {
        format!("{bin_name}.exe")
    } else {
        bin_name
    };
    server_path
        .join("target")
        .join("release")
        .join(bin_filename)
}

// ── Install orchestration ───────────────────────────────────────────

/// Dispatch a marketplace install based on the catalog entry's
/// `install.source`. `None` falls back to the legacy
/// `clotohub-servers` monorepo tarball path so pre-v0.2 registries keep
/// working unchanged. Per-source installers handle materialization, then
/// share [`build_and_register`] / [`register_server`] for the toolchain
/// + add_server + auto_start steps.
async fn run_install(
    state: &AppState,
    entry: &RegistryEntry,
    env_overrides: HashMap<String, String>,
    auto_start: bool,
) -> anyhow::Result<()> {
    use mgp_sdk::adapters::SourceSpec;
    match entry.install.as_ref().map(|i| &i.source) {
        Some(SourceSpec::Git(spec)) => {
            install_from_git(state, entry, spec, env_overrides, auto_start).await
        }
        Some(SourceSpec::RawUrl(spec)) => {
            install_from_raw_url(state, entry, spec, env_overrides, auto_start).await
        }
        Some(SourceSpec::Pypi(spec)) => {
            install_from_pypi(state, entry, spec, env_overrides, auto_start).await
        }
        Some(SourceSpec::Docker(spec)) => {
            install_from_docker(state, entry, spec, env_overrides, auto_start).await
        }
        None => install_from_monorepo_tarball(state, entry, env_overrides, auto_start).await,
    }
}

/// Ensure the toolchain required by `entry.runtime` is available. Emits a
/// `StepError` and returns `Ok(false)` on failure (caller should return
/// early); returns `Ok(true)` when the toolchain is ready.
async fn ensure_toolchain(state: &AppState, is_rust: bool) -> anyhow::Result<bool> {
    let tx = &state.setup_progress_tx;
    if is_rust {
        emit(
            tx,
            SetupProgressEvent::StepStart {
                step: "check_cargo".into(),
                description: "Checking Rust toolchain".into(),
            },
        );
        let (cargo_available, _) = detect_cargo();
        if !cargo_available {
            emit(
                tx,
                SetupProgressEvent::StepError {
                    step: "check_cargo".into(),
                    error: "Rust toolchain (cargo) is required. Install via https://rustup.rs/"
                        .into(),
                    recoverable: false,
                },
            );
            return Ok(false);
        }
        emit(
            tx,
            SetupProgressEvent::StepComplete {
                step: "check_cargo".into(),
            },
        );
    } else {
        emit(
            tx,
            SetupProgressEvent::StepStart {
                step: "check_uv".into(),
                description: "Setting up uv package manager".into(),
            },
        );
        let uv = crate::managers::mcp_venv::uv_bin(&state.data_dir);
        if !uv.exists() {
            info!("Single install: uv not found, downloading...");
            if let Err(e) = super::setup::download_uv(tx, &state.data_dir).await {
                emit(
                    tx,
                    SetupProgressEvent::StepError {
                        step: "check_uv".into(),
                        error: format!("Failed to download uv: {e}"),
                        recoverable: true,
                    },
                );
                return Ok(false);
            }
        }
        emit(
            tx,
            SetupProgressEvent::StepComplete {
                step: "check_uv".into(),
            },
        );
    }
    Ok(true)
}

/// Steps 4-5 for installs that produce a source tree on disk: build /
/// install Python deps from `server_path`, then register via
/// [`register_server`]. Callers must have already invoked
/// [`ensure_toolchain`].
#[allow(clippy::too_many_lines)]
async fn build_and_register(
    state: &AppState,
    entry: &RegistryEntry,
    server_path: &std::path::Path,
    needs_common: bool,
    env_overrides: HashMap<String, String>,
    auto_start: bool,
) -> anyhow::Result<()> {
    let tx = &state.setup_progress_tx;
    let is_rust = entry.runtime == "rust";
    let servers_dir = resolve_servers_dir(state);

    // Step 4: Install dependencies / build
    let (command, args) = if is_rust {
        // ── Rust server: cargo build ──
        // Verify extraction produced a valid Cargo project
        let cargo_toml = server_path.join("Cargo.toml");
        if !cargo_toml.exists() {
            warn!(
                "Extracted directory missing Cargo.toml: {}",
                server_path.display()
            );
            emit(
                tx,
                SetupProgressEvent::StepError {
                    step: "cargo_build".into(),
                    error: format!(
                        "Cargo.toml not found in {}. Extraction may have failed.",
                        server_path.display()
                    ),
                    recoverable: false,
                },
            );
            return Ok(());
        }
        // Ensure the package is not absorbed by a parent workspace.
        // Append [workspace] to Cargo.toml so Cargo treats it as standalone.
        {
            let content = std::fs::read_to_string(&cargo_toml).unwrap_or_else(|e| {
                tracing::warn!(path = %cargo_toml.display(), "Failed to read Cargo.toml: {e}");
                String::new()
            });
            if !content.contains("[workspace]") {
                std::fs::write(&cargo_toml, format!("{content}\n[workspace]\n"))
                    .unwrap_or_else(|e| warn!("Failed to patch Cargo.toml: {e}"));
            }
        }
        info!(
            "Cargo.toml found, starting build in {}",
            server_path.display()
        );

        if !cargo_build_server(tx, server_path, &entry.name).await? {
            return Ok(());
        }

        let bin_path = rust_binary_path(server_path, entry);
        if !bin_path.exists() {
            emit(
                tx,
                SetupProgressEvent::StepError {
                    step: "cargo_build".into(),
                    error: format!("Binary not found at {}", bin_path.display()),
                    recoverable: false,
                },
            );
            return Ok(());
        }

        emit(
            tx,
            SetupProgressEvent::ServerInstall {
                server_name: entry.name.clone(),
                status: "installed".into(),
            },
        );

        (bin_path.to_string_lossy().to_string(), vec![])
    } else {
        // ── Python server: venv + pip install ──
        emit(
            tx,
            SetupProgressEvent::StepStart {
                step: "install_deps".into(),
                description: format!("Installing {} dependencies", entry.name),
            },
        );

        let venv_dir = crate::managers::mcp_venv::resolve_venv_dir()
            .unwrap_or_else(|| servers_dir.join(".venv"));

        // bug-453: serialize the venv check → create → install against the boot
        // sync task and other installs sharing the same .venv.
        let _venv_guard = crate::managers::mcp_venv::lock_venv().await;

        // Ensure venv exists (with timeout to avoid hanging)
        let uv = crate::managers::mcp_venv::uv_bin(&state.data_dir);
        let uv_str = uv.to_string_lossy().to_string();
        let target_python = crate::managers::mcp_venv::TARGET_PYTHON;
        if !venv_dir.join("pyvenv.cfg").exists() {
            let venv_path_str = venv_dir.to_string_lossy().to_string();
            let _ = tokio::time::timeout(
                Duration::from_secs(CHILD_PROCESS_TIMEOUT_SECS),
                tokio::process::Command::new(&uv_str)
                    .args(["venv", "--python", target_python, &venv_path_str])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .status(),
            )
            .await;
        }

        let venv_python = crate::managers::mcp_venv::venv_python(&venv_dir);
        let venv_python_str = venv_python.to_string_lossy().to_string();

        // Install common first if needed (monorepo path only — standalone
        // connectors are expected to bundle their own utility code).
        if needs_common {
            // bug-389/390: the shared `common` package sits flat next to the
            // server dir for legacy monorepo-tarball extraction, or nested
            // inside a git clone (`<clone>/servers/common`). Probe both and
            // require build metadata (pyproject.toml ships with
            // clotohub-servers >= v0.1.8) so `uv pip install` can place the
            // package in the shared venv instead of silently skipping.
            let common_src = resolve_common_source(server_path, &servers_dir);
            if let Some(common_path) = common_src {
                emit(
                    tx,
                    SetupProgressEvent::ServerInstall {
                        server_name: "common".into(),
                        status: "installing".into(),
                    },
                );
                // bug-369: drive the common install through null-stdio status()
                // (matching the batch path) instead of a piped output(). Capturing
                // both pipes without concurrently draining them risks a back-pressure
                // deadlock when uv emits verbose output.
                let mut cmd = tokio::process::Command::new(&uv_str);
                cmd.args([
                    "pip",
                    "install",
                    "--no-progress",
                    "--python",
                    &venv_python_str,
                    &common_path.to_string_lossy(),
                ])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true); // bug-366: reap on install-task abort
                #[cfg(windows)]
                cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
                let result = tokio::time::timeout(
                    Duration::from_secs(CHILD_PROCESS_TIMEOUT_SECS),
                    cmd.status(),
                )
                .await;
                match result {
                    Ok(Ok(status)) if status.success() => {
                        emit(
                            tx,
                            SetupProgressEvent::ServerInstall {
                                server_name: "common".into(),
                                status: "installed".into(),
                            },
                        );
                    }
                    Err(_) => warn!("Common module install timed out (120s)"),
                    _ => warn!("Failed to install common dependency"),
                }
            } else {
                warn!(
                    "Connector '{}' declares a 'common' dependency but no installable \
                     common package (with pyproject.toml) was found next to {} or in {} \
                     — the server may fail with ModuleNotFoundError: No module named 'common'",
                    entry.id,
                    server_path.display(),
                    servers_dir.display()
                );
            }
        }

        // Install the target server
        emit(
            tx,
            SetupProgressEvent::ServerInstall {
                server_name: entry.name.clone(),
                status: "installing".into(),
            },
        );

        // bug-369: stream the server install through spawn_uv_streaming, which
        // drains stdout/stderr on dedicated tasks and enforces a timeout. The
        // previous piped output() had no timeout and could deadlock on a full pipe.
        let server_path_str = server_path.to_string_lossy().to_string();
        let logs_dir = state.data_dir.join("logs");
        let _ = tokio::fs::create_dir_all(&logs_dir).await;
        let log_file = logs_dir.join("install.log");
        match super::setup::spawn_uv_streaming(
            tx,
            &uv_str,
            &venv_python_str,
            &entry.name,
            &server_path_str,
            Some(&log_file),
        )
        .await
        {
            Ok(()) => {
                emit(
                    tx,
                    SetupProgressEvent::ServerInstall {
                        server_name: entry.name.clone(),
                        status: "installed".into(),
                    },
                );
            }
            Err(detail) => {
                emit(
                    tx,
                    SetupProgressEvent::StepError {
                        step: "install_deps".into(),
                        error: format!("uv pip install failed: {detail}"),
                        recoverable: true,
                    },
                );
                return Ok(());
            }
        }

        emit(
            tx,
            SetupProgressEvent::StepComplete {
                step: "install_deps".into(),
            },
        );

        let server_script = server_path.join("server.py").to_string_lossy().to_string();
        // Use bare "python" — resolved to venv python at spawn time
        ("python".to_string(), vec![server_script])
    };

    register_server(state, entry, command, args, env_overrides, auto_start).await
}

/// bug-394 proper fix: decide the install-time seal from the catalog
/// entry's cryptographic evidence (D2 bifurcation — see
/// `project_clotohub_ed25519_seal_verification.md` §5).
///
/// The catalog seal is HMAC-signed with the *hub* master key and can
/// never verify against this kernel's local seal key, so it is never
/// passed to the spawn-time check. Instead:
///
/// - **Verified** — the entry carries an Ed25519 signature whose key we
///   resolved from the hub JWKS, the signature validates over
///   `canonical_message(id, version, entry_point_sha256)`, and the
///   installed entry point hashes to that `entry_point_sha256` → mint a
///   LOCAL seal (declared trust tier preserved; every later spawn check
///   verifies under the local protocol).
/// - **Benign non-verification** (`Ok(None)` → register unsealed; MGP
///   §10 invariant 3 forces the untrusted profile at spawn): no
///   signature (pre-Ed25519 catalog), malformed signature block, or
///   JWKS unreachable / kid unknown (offline, rotation lag).
/// - **Tamper suspect** (`Err` → install hard-blocks, loud audit log):
///   a resolvable, well-formed signature that fails verification
///   (the signed identity does not match the served entry), or an
///   entry-point hash mismatch (the delivered bytes do not match what
///   was signed / recorded).
///
/// `hub_key` is the signing key pre-resolved for the entry's `kid` via
/// [`hub_signing_key`]; `None` means the JWKS path is unavailable.
fn local_seal_for_install(
    data_dir: &std::path::Path,
    entry: &RegistryEntry,
    command: &str,
    args: &[String],
    hub_key: Option<&mgp_seal::ed25519::PublicKey>,
) -> anyhow::Result<Option<String>> {
    let ed25519_block = entry
        .signature_payload
        .as_ref()
        .and_then(|p| p.get("ed25519"));

    let Some(expected) = entry
        .entry_point_sha256
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        if ed25519_block.is_some() {
            warn!(
                "{}: catalog entry carries a signature but no entry_point_sha256 — \
                 cannot verify; registering unsealed (untrusted at spawn)",
                entry.id
            );
        }
        return Ok(None);
    };

    // Keyless integrity check first: a hash mismatch is the strongest
    // tamper signal regardless of signature state, and hard-blocks
    // (unchanged from the interim fix).
    let entry_point = crate::managers::mcp::resolve_sealable_entry_point(command, args);
    let data = std::fs::read(entry_point).map_err(|e| {
        anyhow::anyhow!(
            "read entry point for integrity check ({}): {e}",
            entry_point.display()
        )
    })?;
    let actual = {
        use sha2::Digest;
        hex::encode(sha2::Sha256::digest(&data))
    };
    if !actual.eq_ignore_ascii_case(expected) {
        error!(
            "{}: TAMPER SUSPECT (integrity_mismatch) — catalog records sha256 {expected}, \
             installed entry point hashes to {actual}",
            entry.id
        );
        anyhow::bail!(
            "entry point integrity check failed for '{}': catalog expects sha256 {expected}, installed file hashes to {actual}",
            entry.id
        );
    }

    // Ed25519 layer: upgrade the trust anchor from "HTTPS to the
    // catalog" to "the hub's signing key".
    let Some(block) = ed25519_block else {
        info!(
            "{}: no Ed25519 signature on catalog entry — registering unsealed \
             (untrusted at spawn)",
            entry.id
        );
        return Ok(None);
    };
    let (Some(sig_b64), Some(kid_str)) = (
        block.get("sig").and_then(serde_json::Value::as_str),
        block.get("key_id").and_then(serde_json::Value::as_str),
    ) else {
        warn!(
            "{}: malformed ed25519 block in signature_payload — registering unsealed",
            entry.id
        );
        return Ok(None);
    };
    let (Ok(sig), Ok(kid)) = (
        mgp_seal::ed25519::Signature::from_base64(sig_b64),
        mgp_seal::ed25519::KeyId::new(kid_str),
    ) else {
        warn!(
            "{}: undecodable Ed25519 signature or key id — registering unsealed",
            entry.id
        );
        return Ok(None);
    };
    let Some(pk) = hub_key else {
        warn!(
            "{}: hub JWKS unavailable or key '{kid_str}' unknown — cannot verify seal \
             signature; registering unsealed (untrusted at spawn)",
            entry.id
        );
        return Ok(None);
    };

    let canonical = mgp_seal::canonical_message(&entry.id, &entry.version, expected);
    if !mgp_seal::ed25519::verify(pk, &kid, &canonical, &sig) {
        error!(
            "{}: TAMPER SUSPECT (signature_invalid) — Ed25519 signature under hub key \
             '{kid_str}' does not match the entry's (id, version, entry_point_sha256)",
            entry.id
        );
        anyhow::bail!(
            "Ed25519 seal verification failed for '{}': the catalog's signature does not match its (id, version, entry_point_sha256) under hub key '{kid_str}' — refusing install",
            entry.id
        );
    }

    let seal_key = mgp_seal::load_or_generate_seal_key(data_dir)?;
    let seal = mgp_seal::compute_seal(entry_point, &seal_key)?;
    info!(
        "{}: Ed25519 seal verified under hub key '{kid_str}'; minted local seal at \
         declared tier '{}'",
        entry.id, entry.trust_level
    );
    Ok(Some(seal))
}

/// Step 5: register the materialized server via `add_server`, persist
/// marketplace metadata, and honor `auto_start`. Independent of how the
/// source was materialized.
async fn register_server(
    state: &AppState,
    entry: &RegistryEntry,
    command: String,
    args: Vec<String>,
    env_overrides: HashMap<String, String>,
    auto_start: bool,
) -> anyhow::Result<()> {
    let tx = &state.setup_progress_tx;

    emit(
        tx,
        SetupProgressEvent::StepStart {
            step: "finalize".into(),
            description: "Registering server".into(),
        },
    );

    // Build env: merge defaults with overrides
    let mut env_map: HashMap<String, String> = HashMap::new();
    for var in &entry.env_vars {
        if let Some(default) = &var.default {
            env_map.insert(var.name.clone(), default.clone());
        }
    }
    for (k, v) in &env_overrides {
        env_map.insert(k.clone(), v.clone());
    }

    // Use add_server() for proper lifecycle integration:
    // creates ServerConfig → connect_server() (spawn + register) → save to DB.
    // The registry's trust_level is threaded through as MgpServerConfig so
    // isolation derivation at next boot picks up the correct level. The
    // hub's seal is NOT stored (bug-394: it is unverifiable against the
    // local seal key); instead the entry's Ed25519 signature is verified
    // against the hub JWKS, the entry point is integrity-checked against
    // the signed entry_point_sha256, and a local seal is minted on success.
    let hub_key = match entry_signature_kid(entry) {
        Some(kid) => hub_signing_key(state, &kid).await,
        None => None,
    };
    let seal =
        match local_seal_for_install(&state.data_dir, entry, &command, &args, hub_key.as_ref()) {
            Ok(seal) => seal,
            Err(e) => {
                emit(
                    tx,
                    SetupProgressEvent::StepError {
                        step: "finalize".into(),
                        error: e.to_string(),
                        recoverable: false,
                    },
                );
                return Ok(());
            }
        };
    let mgp = Some(crate::managers::mcp_mgp::MgpServerConfig {
        trust_level: Some(entry.trust_level.clone()),
    });
    match state
        .mcp_manager
        .add_server(
            entry.id.clone(),
            command,
            args,
            None,
            Some(entry.description.clone()),
            mgp,
            seal,
            env_map,
        )
        .await
    {
        Ok(tools) => {
            info!(
                "Marketplace server connected: {} ({} tools)",
                entry.id,
                tools.len()
            );
        }
        Err(e) => {
            warn!("Server registered but failed to connect: {e}");
            // Continue — server is in DB and can be started manually later
        }
    }

    // Set marketplace-specific fields (source, version, marketplace_id)
    if let Err(e) = crate::db::mcp::set_marketplace_fields(
        &state.pool,
        &entry.id,
        &entry.version,
        &entry.id,
        Some(&entry.trust_level),
    )
    .await
    {
        warn!("Failed to set marketplace fields: {e}");
    }

    // Engine provider metadata (Goal #148): a reasoning-engine connector
    // (`category = "mind"`) carries its upstream LLM-provider metadata in the
    // catalog entry's `provider` block. Ingest it into the `llm_providers`
    // credential registry so a new engine needs only a catalog entry, not a
    // seed migration. Meta columns only — user-owned key/model/context/thinking
    // are never overwritten (db::upsert_llm_provider_meta).
    if let Some(pm) = &entry.provider {
        let quirks = crate::db::ProviderQuirks {
            no_api_key: pm.quirks.as_ref().is_some_and(|q| q.no_api_key),
            models_endpoint_path: pm
                .quirks
                .as_ref()
                .and_then(|q| q.models_endpoint_path.clone()),
            switch_model_tool: pm.quirks.as_ref().and_then(|q| q.switch_model_tool.clone()),
            model_placeholder: pm.model_placeholder.clone(),
        };
        let quirks_json = serde_json::to_string(&quirks).ok();
        if let Err(e) = crate::db::upsert_llm_provider_meta(
            &state.pool,
            &entry.id,
            &entry.name,
            &pm.api_url,
            &pm.auth_type,
            pm.model_default.as_deref().unwrap_or(""),
            pm.timeout_secs.map_or(120, |t| t as i32),
            quirks_json,
        )
        .await
        {
            warn!(
                "Failed to ingest LLM provider metadata for {}: {e}",
                entry.id
            );
        }
    }

    // A reasoning engine installed here is auto-granted to any agent that
    // already declares it as `default_engine_id`, as an explicit server_grant.
    // This is the install-time complement of the create_agent handler and the
    // capability-gate backfill migration (which could only grant engines already
    // installed when it ran): without it, a seeded agent whose hardcoded default
    // engine is installed later returned MGP-1001 on its own default engine until
    // granted by hand. Scoped to `default_engine_id` only; a non-clobber guard in
    // the query preserves any explicit deny. See MCP_CAPABILITY_GATE_DESIGN.md §6.
    if entry.category == "mind" {
        match crate::db::mcp::grant_default_engine_to_agents(&state.pool, &entry.id).await {
            Ok(n) if n > 0 => info!(
                "Auto-granted engine '{}' to {} agent(s) with a matching default_engine_id",
                entry.id, n
            ),
            Ok(_) => {}
            Err(e) => warn!(
                "Failed to auto-grant engine '{}' to its default-engine agents: {e}",
                entry.id
            ),
        }
    }

    // Modern decouple path (0.6.6+): when a memory-kind plugin is installed
    // and the default agent has not yet picked one, mirror this install id
    // into `agent.cloto_default.metadata.preferred_memory`. This replaces
    // the pre-0.6.6 path where the kernel hardcoded a memory plugin id via
    // CLOTO_MEMORY_PLUGIN_ID env default and seeded plugin_configs at boot.
    // Idempotent — touches nothing if the user already chose a memory
    // plugin manually via the dashboard.
    if entry.category == "memory" {
        if let Err(e) = bind_default_memory_if_unset(&state.pool, &entry.id).await {
            warn!("Failed to bind default agent's preferred_memory: {e}");
        }
    }

    // If user requested no auto-start, stop the server after registration
    if !auto_start {
        let _ = state.mcp_manager.stop_server(&entry.id).await;
    }

    emit(
        tx,
        SetupProgressEvent::StepComplete {
            step: "finalize".into(),
        },
    );

    info!("Marketplace install complete: {}", entry.id);
    Ok(())
}

/// Set `agent.cloto_default.metadata.preferred_memory` to the given plugin
/// id iff currently absent or empty. Idempotent — touches nothing if the
/// user has already chosen a memory plugin manually.
///
/// Used by `register_server` to auto-bind a memory plugin install when no
/// kernel-side hardcode is doing it (post-0.6.6 decouple). Mirrors the
/// shape of the Phase C migration that back-fills the same key for
/// pre-0.6.6 users (`migrations/<…>_backfill_default_memory.sql`).
async fn bind_default_memory_if_unset(
    pool: &sqlx::SqlitePool,
    plugin_id: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE agents \
         SET metadata = json_set(COALESCE(metadata, '{}'), '$.preferred_memory', ?) \
         WHERE id = 'agent.cloto_default' \
           AND ( \
             json_extract(metadata, '$.preferred_memory') IS NULL \
             OR json_extract(metadata, '$.preferred_memory') = '' \
           )",
    )
    .bind(plugin_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Legacy install path: download the entire `clotohub-servers` monorepo
/// tarball, selectively extract `{entry.directory}/` (plus `common/` if
/// depended on), then build_and_register. Used when the catalog entry
/// carries no per-entry `install.source` block — preserves backward
/// compat with the pre-v0.2 registry.json shape.
#[allow(clippy::too_many_lines)]
async fn install_from_monorepo_tarball(
    state: &AppState,
    entry: &RegistryEntry,
    env_overrides: HashMap<String, String>,
    auto_start: bool,
) -> anyhow::Result<()> {
    let tx = &state.setup_progress_tx;
    let is_rust = entry.runtime == "rust";

    if !ensure_toolchain(state, is_rust).await? {
        return Ok(());
    }

    // Step 2: Download repo tarball
    emit(
        tx,
        SetupProgressEvent::StepStart {
            step: "download".into(),
            description: format!("Downloading {} from repository", entry.name),
        },
    );

    let tmp_dir = state.data_dir.join("tmp");
    tokio::fs::create_dir_all(&tmp_dir).await?;
    let archive_path = tmp_dir.join("cloto-mcp-servers-latest.tar.gz");

    let tarball_url = tarball_url("master");

    // Download with custom headers for GitHub API
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(TARBALL_DOWNLOAD_TIMEOUT_SECS))
        .build()?;
    let resp = client
        .get(tarball_url)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "ClotoCore")
        .send()
        .await?;

    if !resp.status().is_success() {
        emit(
            tx,
            SetupProgressEvent::StepError {
                step: "download".into(),
                error: format!("GitHub API returned HTTP {}", resp.status()),
                recoverable: true,
            },
        );
        return Ok(());
    }

    // Stream download to file
    let total = resp.content_length();
    let mut stream = resp.bytes_stream();
    let mut file = tokio::fs::File::create(&archive_path).await?;
    let mut downloaded: u64 = 0;

    use futures::StreamExt;
    use tokio::io::AsyncWriteExt;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;

        if let Some(total) = total {
            let progress = (downloaded as f32 / total as f32).min(1.0);
            let mb_done = downloaded as f64 / 1_048_576.0;
            let mb_total = total as f64 / 1_048_576.0;
            emit(
                tx,
                SetupProgressEvent::StepProgress {
                    step: "download".into(),
                    progress,
                    detail: format!("{mb_done:.1} / {mb_total:.1} MB"),
                },
            );
        }
    }
    file.flush().await?;

    emit(
        tx,
        SetupProgressEvent::StepComplete {
            step: "download".into(),
        },
    );

    // Step 3: Selective extraction
    emit(
        tx,
        SetupProgressEvent::StepStart {
            step: "extract".into(),
            description: format!("Extracting {}", entry.name),
        },
    );

    let servers_dir = resolve_servers_dir(state);
    let directory = entry.directory.clone();
    // bug-389: always provision `common` when declared — a stale flat copy on
    // disk says nothing about whether the shared venv can import it.
    let needs_common = !is_rust && entry.dependencies.contains(&"common".to_string());

    let archive_path_clone = archive_path.clone();
    let servers_dir_clone = servers_dir.clone();

    tokio::task::spawn_blocking(move || {
        extract_selective(
            &archive_path_clone,
            &servers_dir_clone,
            &directory,
            needs_common,
        )
    })
    .await??;

    emit(
        tx,
        SetupProgressEvent::StepComplete {
            step: "extract".into(),
        },
    );

    let server_path = servers_dir.join(&entry.directory);
    let result = build_and_register(
        state,
        entry,
        &server_path,
        needs_common,
        env_overrides,
        auto_start,
    )
    .await;

    // Cleanup tarball regardless of register outcome
    if let Err(e) = tokio::fs::remove_file(&archive_path).await {
        warn!("Failed to cleanup archive {}: {e}", archive_path.display());
    }

    result
}

/// Install from a `git` source: clone `{spec.url}` into
/// `{servers_dir}/{entry.directory}`, optionally checking out
/// `spec.reference`. When `spec.subdir` is set, the actual server path
/// becomes `{clone_dir}/{spec.subdir}`. Requires `git` on PATH; emits a
/// clear StepError if missing.
#[allow(clippy::too_many_lines)]
async fn install_from_git(
    state: &AppState,
    entry: &RegistryEntry,
    spec: &mgp_sdk::adapters::GitSpec,
    env_overrides: HashMap<String, String>,
    auto_start: bool,
) -> anyhow::Result<()> {
    let tx = &state.setup_progress_tx;
    let is_rust = entry.runtime == "rust";

    if !ensure_toolchain(state, is_rust).await? {
        return Ok(());
    }

    emit(
        tx,
        SetupProgressEvent::StepStart {
            step: "git_clone".into(),
            description: format!("Cloning {} from {}", entry.name, spec.url),
        },
    );

    let servers_dir = resolve_servers_dir(state);
    tokio::fs::create_dir_all(&servers_dir).await?;
    let clone_dir = match resolve_install_dir(&servers_dir, entry) {
        Ok(dir) => dir,
        Err(e) => {
            emit(
                tx,
                SetupProgressEvent::StepError {
                    step: "git_clone".into(),
                    error: e.to_string(),
                    recoverable: false,
                },
            );
            return Ok(());
        }
    };

    // Clear any leftover tree from a prior failed install so `git clone`
    // can target a fresh directory.
    if clone_dir.exists() {
        if let Err(e) = tokio::fs::remove_dir_all(&clone_dir).await {
            warn!(
                "Failed to clear existing clone dir {}: {e}",
                clone_dir.display()
            );
        }
    }

    // `--depth 1 --branch <ref>` covers branches and tags; full commit
    // SHAs are not supported in v1 because `git clone --branch` rejects
    // SHA arguments and a fallback to fetch-then-checkout would double
    // the network cost on the happy path. Connectors that need to pin a
    // SHA should publish a tag pointing at that commit instead.
    let mut clone_cmd = tokio::process::Command::new("git");
    clone_cmd.arg("clone").arg("--depth").arg("1");
    if !spec.reference.is_empty() {
        clone_cmd.arg("--branch").arg(&spec.reference);
    }
    clone_cmd
        .arg(&spec.url)
        .arg(&clone_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let clone_result = tokio::time::timeout(
        Duration::from_secs(TARBALL_DOWNLOAD_TIMEOUT_SECS),
        clone_cmd.output(),
    )
    .await;

    match clone_result {
        Ok(Ok(output)) if output.status.success() => {
            emit(
                tx,
                SetupProgressEvent::StepComplete {
                    step: "git_clone".into(),
                },
            );
        }
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let last_line = stderr.lines().last().unwrap_or("git clone failed");
            emit(
                tx,
                SetupProgressEvent::StepError {
                    step: "git_clone".into(),
                    error: format!("git clone failed: {last_line}"),
                    recoverable: true,
                },
            );
            return Ok(());
        }
        Ok(Err(e)) => {
            emit(
                tx,
                SetupProgressEvent::StepError {
                    step: "git_clone".into(),
                    error: format!(
                        "Failed to spawn git: {e}. Ensure git is installed and on PATH."
                    ),
                    recoverable: false,
                },
            );
            return Ok(());
        }
        Err(_) => {
            emit(
                tx,
                SetupProgressEvent::StepError {
                    step: "git_clone".into(),
                    error: "git clone timed out".into(),
                    recoverable: true,
                },
            );
            return Ok(());
        }
    }

    let server_path = match spec.subdir.as_deref().filter(|s| !s.is_empty()) {
        Some(subdir) => clone_dir.join(subdir),
        None => clone_dir,
    };
    // bug-389: a nested clone never has `common` at the flat location, so the
    // old flat-layout probe always re-detected "missing"; conversely a stale
    // flat copy must not suppress the venv install. Declared dependency is
    // the only signal that matters.
    let needs_common = !is_rust && entry.dependencies.contains(&"common".to_string());

    build_and_register(
        state,
        entry,
        &server_path,
        needs_common,
        env_overrides,
        auto_start,
    )
    .await
}

/// Install from a single HTTP(S) artifact: download `{spec.url}`,
/// verify `spec.sha256` if present, extract a `.tar.gz` into
/// `{servers_dir}/{entry.directory}` (stripping a single shared
/// top-level prefix when the archive uses one, GitHub-style), then
/// build_and_register.
#[allow(clippy::too_many_lines)]
async fn install_from_raw_url(
    state: &AppState,
    entry: &RegistryEntry,
    spec: &mgp_sdk::adapters::RawUrlSpec,
    env_overrides: HashMap<String, String>,
    auto_start: bool,
) -> anyhow::Result<()> {
    let tx = &state.setup_progress_tx;
    let is_rust = entry.runtime == "rust";

    if !ensure_toolchain(state, is_rust).await? {
        return Ok(());
    }

    emit(
        tx,
        SetupProgressEvent::StepStart {
            step: "download".into(),
            description: format!("Downloading {} from {}", entry.name, spec.url),
        },
    );

    // bug-431: `spec.url` is hub-served catalog data. Validate its shape up front
    // (scheme http/https + sha256 hex — the check the docker path already runs for
    // DockerSpec) and route the download through the same private-IP / loopback
    // SSRF guard the rest of the codebase enforces
    // (capabilities::resolve_unrestricted_addrs): resolve the host, reject any
    // restricted IP, then PIN the connection to the validated addresses with
    // redirects disabled so a whitelisted-looking host cannot DNS-rebind or 3xx to
    // cloud-metadata / loopback. The LLM outbound whitelist is intentionally NOT
    // applied here — marketplace hosts are authorized by catalog provenance.
    if let Err(e) = spec.check() {
        emit(
            tx,
            SetupProgressEvent::StepError {
                step: "download".into(),
                error: format!("Invalid raw_url source: {e}"),
                recoverable: false,
            },
        );
        return Ok(());
    }
    let parsed_url = match reqwest::Url::parse(&spec.url) {
        Ok(u) => u,
        Err(e) => {
            emit(
                tx,
                SetupProgressEvent::StepError {
                    step: "download".into(),
                    error: format!("Invalid raw_url URL: {e}"),
                    recoverable: false,
                },
            );
            return Ok(());
        }
    };
    let Some(host) = parsed_url.host_str().map(str::to_string) else {
        emit(
            tx,
            SetupProgressEvent::StepError {
                step: "download".into(),
                error: "raw_url has no host".into(),
                recoverable: false,
            },
        );
        return Ok(());
    };
    let port = parsed_url.port_or_known_default().unwrap_or(443);
    let resolved = match crate::capabilities::resolve_unrestricted_addrs(&host, port).await {
        Ok(addrs) => addrs,
        Err(e) => {
            emit(
                tx,
                SetupProgressEvent::StepError {
                    step: "download".into(),
                    error: e.to_string(),
                    recoverable: false,
                },
            );
            return Ok(());
        }
    };

    let tmp_dir = state.data_dir.join("tmp");
    tokio::fs::create_dir_all(&tmp_dir).await?;
    let archive_path = tmp_dir.join(format!("{}-raw-url.tar.gz", entry.id));

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(TARBALL_DOWNLOAD_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(&host, &resolved)
        .build()?;
    let resp = client
        .get(parsed_url)
        .header("User-Agent", "ClotoCore")
        .send()
        .await?;

    if !resp.status().is_success() {
        emit(
            tx,
            SetupProgressEvent::StepError {
                step: "download".into(),
                error: format!("HTTP {} from {}", resp.status(), spec.url),
                recoverable: true,
            },
        );
        return Ok(());
    }

    let total = resp.content_length();
    let mut stream = resp.bytes_stream();
    let mut file = tokio::fs::File::create(&archive_path).await?;
    let mut downloaded: u64 = 0;
    let mut hasher = sha2::Sha256::new();

    use futures::StreamExt;
    use sha2::Digest;
    use tokio::io::AsyncWriteExt;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        if spec.sha256.is_some() {
            hasher.update(&chunk);
        }
        downloaded += chunk.len() as u64;

        if let Some(total) = total {
            let progress = (downloaded as f32 / total as f32).min(1.0);
            let mb_done = downloaded as f64 / 1_048_576.0;
            let mb_total = total as f64 / 1_048_576.0;
            emit(
                tx,
                SetupProgressEvent::StepProgress {
                    step: "download".into(),
                    progress,
                    detail: format!("{mb_done:.1} / {mb_total:.1} MB"),
                },
            );
        }
    }
    file.flush().await?;

    if let Some(expected) = &spec.sha256 {
        let actual = hex::encode(hasher.finalize());
        if !actual.eq_ignore_ascii_case(expected) {
            // bug-367: surface cleanup failures instead of swallowing them, so a
            // corrupted-download retry loop can't silently leak disk space.
            if let Err(e) = tokio::fs::remove_file(&archive_path).await {
                warn!(
                    "Failed to cleanup archive after sha256 mismatch {}: {e}",
                    archive_path.display()
                );
            }
            emit(
                tx,
                SetupProgressEvent::StepError {
                    step: "download".into(),
                    error: format!("sha256 mismatch: expected {expected}, got {actual}"),
                    recoverable: false,
                },
            );
            return Ok(());
        }
    }

    emit(
        tx,
        SetupProgressEvent::StepComplete {
            step: "download".into(),
        },
    );

    emit(
        tx,
        SetupProgressEvent::StepStart {
            step: "extract".into(),
            description: format!("Extracting {}", entry.name),
        },
    );

    let servers_dir = resolve_servers_dir(state);
    tokio::fs::create_dir_all(&servers_dir).await?;
    let target_dir = match resolve_install_dir(&servers_dir, entry) {
        Ok(dir) => dir,
        Err(e) => {
            emit(
                tx,
                SetupProgressEvent::StepError {
                    step: "extract".into(),
                    error: e.to_string(),
                    recoverable: false,
                },
            );
            return Ok(());
        }
    };
    if target_dir.exists() {
        if let Err(e) = tokio::fs::remove_dir_all(&target_dir).await {
            warn!(
                "Failed to clear existing target dir {}: {e}",
                target_dir.display()
            );
        }
    }
    tokio::fs::create_dir_all(&target_dir).await?;

    // Goal #110: a `raw_url` source may carry a `subdir` (mgp-sdk
    // v0.3.0) when the tarball is a whole monorepo served by the
    // ClotoHub blob mirror. With a subdir we extract only the
    // connector's tree (plus the sibling `common/` package when
    // declared), preserving repo-relative paths so the on-disk layout
    // matches a nested git clone — `resolve_common_source` and
    // uninstall both already understand that shape. Without a subdir
    // the tarball is standalone and extracts whole, as before.
    let subdir = spec
        .subdir
        .clone()
        .filter(|s| !s.is_empty())
        .map(|s| s.trim_matches('/').to_owned());
    let needs_common = !is_rust && entry.dependencies.contains(&"common".to_string());

    let archive_path_clone = archive_path.clone();
    let target_dir_clone = target_dir.clone();
    let subdir_clone = subdir.clone();
    tokio::task::spawn_blocking(move || match &subdir_clone {
        Some(sub) => {
            extract_subdir_selective(&archive_path_clone, &target_dir_clone, sub, needs_common)
        }
        None => extract_tarball_stripped(&archive_path_clone, &target_dir_clone),
    })
    .await??;

    emit(
        tx,
        SetupProgressEvent::StepComplete {
            step: "extract".into(),
        },
    );

    let server_path = match &subdir {
        Some(sub) => target_dir.join(sub),
        None => target_dir.clone(),
    };
    let result = build_and_register(
        state,
        entry,
        &server_path,
        needs_common,
        env_overrides,
        auto_start,
    )
    .await;

    if let Err(e) = tokio::fs::remove_file(&archive_path).await {
        warn!("Failed to cleanup archive {}: {e}", archive_path.display());
    }

    result
}

/// Install from a PyPI package: ensure the shared venv, run
/// `uv pip install {spec.install_argument()}`, then register the
/// connector as `python -m {module}`. `{module}` resolves to
/// `entry.bin_name` (if set) or `entry.id.replace('-', '_')` — avoids
/// invoking a console script directly since the kernel's command
/// whitelist would reject one (see `mcp_transport::ALLOWED_COMMANDS`).
/// Python-only; errors for `entry.runtime == "rust"`.
#[allow(clippy::too_many_lines)]
async fn install_from_pypi(
    state: &AppState,
    entry: &RegistryEntry,
    spec: &mgp_sdk::adapters::PypiSpec,
    env_overrides: HashMap<String, String>,
    auto_start: bool,
) -> anyhow::Result<()> {
    let tx = &state.setup_progress_tx;

    if entry.runtime == "rust" {
        emit(
            tx,
            SetupProgressEvent::StepError {
                step: "check_source".into(),
                error: format!(
                    "Connector '{}' declares source.type=pypi with runtime=rust; pypi installs are Python-only.",
                    entry.id
                ),
                recoverable: false,
            },
        );
        return Ok(());
    }

    if !ensure_toolchain(state, false).await? {
        return Ok(());
    }

    emit(
        tx,
        SetupProgressEvent::StepStart {
            step: "install_deps".into(),
            description: format!("Installing {} from PyPI", entry.name),
        },
    );

    let servers_dir = resolve_servers_dir(state);
    let venv_dir =
        crate::managers::mcp_venv::resolve_venv_dir().unwrap_or_else(|| servers_dir.join(".venv"));

    // bug-453: serialize the venv check → create → install (shared .venv).
    let _venv_guard = crate::managers::mcp_venv::lock_venv().await;

    let uv = crate::managers::mcp_venv::uv_bin(&state.data_dir);
    let uv_str = uv.to_string_lossy().to_string();
    let target_python = crate::managers::mcp_venv::TARGET_PYTHON;
    if !venv_dir.join("pyvenv.cfg").exists() {
        let venv_path_str = venv_dir.to_string_lossy().to_string();
        let _ = tokio::time::timeout(
            Duration::from_secs(CHILD_PROCESS_TIMEOUT_SECS),
            tokio::process::Command::new(&uv_str)
                .args(["venv", "--python", target_python, &venv_path_str])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .status(),
        )
        .await;
    }

    let venv_python = crate::managers::mcp_venv::venv_python(&venv_dir);
    let venv_python_str = venv_python.to_string_lossy().to_string();

    let install_arg = spec.install_argument();
    emit(
        tx,
        SetupProgressEvent::ServerInstall {
            server_name: entry.name.clone(),
            status: "installing".into(),
        },
    );
    let result = tokio::process::Command::new(&uv_str)
        .args([
            "pip",
            "install",
            "--no-progress",
            "--python",
            &venv_python_str,
            &install_arg,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await;

    match result {
        Ok(output) if output.status.success() => {
            emit(
                tx,
                SetupProgressEvent::ServerInstall {
                    server_name: entry.name.clone(),
                    status: "installed".into(),
                },
            );
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let last_line = stderr.lines().last().unwrap_or("unknown error");
            emit(
                tx,
                SetupProgressEvent::StepError {
                    step: "install_deps".into(),
                    error: format!("uv pip install {install_arg} failed: {last_line}"),
                    recoverable: true,
                },
            );
            return Ok(());
        }
        Err(e) => {
            emit(
                tx,
                SetupProgressEvent::StepError {
                    step: "install_deps".into(),
                    error: format!("Failed to run uv pip install: {e}"),
                    recoverable: true,
                },
            );
            return Ok(());
        }
    }

    emit(
        tx,
        SetupProgressEvent::StepComplete {
            step: "install_deps".into(),
        },
    );

    let module = entry
        .bin_name
        .clone()
        .unwrap_or_else(|| entry.id.replace('-', "_"));

    register_server(
        state,
        entry,
        "python".to_string(),
        vec!["-m".to_string(), module],
        env_overrides,
        auto_start,
    )
    .await
}

/// Install a marketplace server whose `install.source` is a Docker image.
/// Verifies the `docker` CLI is on `PATH`, pulls the image, and registers
/// the server as `docker run --rm -i [-e KEY ...] <image>:<tag>`. Env vars
/// declared in `entry.env_vars` plus any `env_overrides` are exposed to the
/// container via `-e KEY` flags (no inline `=VALUE` form, so secrets do not
/// leak into the process listing — values flow through the parent env that
/// `register_server` configures on the docker CLI process).
async fn install_from_docker(
    state: &AppState,
    entry: &RegistryEntry,
    spec: &mgp_sdk::adapters::DockerSpec,
    env_overrides: HashMap<String, String>,
    auto_start: bool,
) -> anyhow::Result<()> {
    let tx = &state.setup_progress_tx;

    if let Err(msg) = spec.check() {
        emit(
            tx,
            SetupProgressEvent::StepError {
                step: "check_source".into(),
                error: format!("Invalid docker source for '{}': {msg}", entry.id),
                recoverable: false,
            },
        );
        return Ok(());
    }

    emit(
        tx,
        SetupProgressEvent::StepStart {
            step: "check_docker".into(),
            description: "Checking docker CLI".into(),
        },
    );

    let docker_check = tokio::time::timeout(
        Duration::from_secs(CHILD_PROCESS_TIMEOUT_SECS),
        tokio::process::Command::new("docker")
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output(),
    )
    .await;

    match docker_check {
        Ok(Ok(output)) if output.status.success() => {
            emit(
                tx,
                SetupProgressEvent::StepComplete {
                    step: "check_docker".into(),
                },
            );
        }
        _ => {
            emit(
                tx,
                SetupProgressEvent::StepError {
                    step: "check_docker".into(),
                    error: "`docker` not found on PATH. Install Docker Desktop or a compatible OCI CLI and retry.".into(),
                    recoverable: true,
                },
            );
            return Ok(());
        }
    }

    let reference = spec.canonical_reference();
    emit(
        tx,
        SetupProgressEvent::StepStart {
            step: "pull_image".into(),
            description: format!("Pulling {reference}"),
        },
    );

    let pull_result = tokio::time::timeout(
        Duration::from_secs(TARBALL_DOWNLOAD_TIMEOUT_SECS),
        tokio::process::Command::new("docker")
            .args(["pull", &reference])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output(),
    )
    .await;

    match pull_result {
        Ok(Ok(output)) if output.status.success() => {
            emit(
                tx,
                SetupProgressEvent::StepComplete {
                    step: "pull_image".into(),
                },
            );
        }
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let last_line = stderr.lines().last().unwrap_or("unknown error");
            emit(
                tx,
                SetupProgressEvent::StepError {
                    step: "pull_image".into(),
                    error: format!("docker pull {reference} failed: {last_line}"),
                    recoverable: true,
                },
            );
            return Ok(());
        }
        Ok(Err(e)) => {
            emit(
                tx,
                SetupProgressEvent::StepError {
                    step: "pull_image".into(),
                    error: format!("Failed to run docker pull: {e}"),
                    recoverable: true,
                },
            );
            return Ok(());
        }
        Err(_) => {
            emit(
                tx,
                SetupProgressEvent::StepError {
                    step: "pull_image".into(),
                    error: format!(
                        "docker pull {reference} timed out after {TARBALL_DOWNLOAD_TIMEOUT_SECS}s"
                    ),
                    recoverable: true,
                },
            );
            return Ok(());
        }
    }

    // Dedup env keys from entry.env_vars (with defaults) ∪ env_overrides
    // so the docker run arg list carries each `-e KEY` once. Values reach
    // the container through the parent env that register_server attaches
    // to the docker CLI process — no `KEY=VALUE` in argv keeps secrets
    // out of `ps`.
    let mut env_keys: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for var in &entry.env_vars {
        if var.default.is_some() {
            env_keys.insert(var.name.clone());
        }
    }
    for k in env_overrides.keys() {
        env_keys.insert(k.clone());
    }

    let mut args = vec!["run".to_string(), "--rm".to_string(), "-i".to_string()];
    for key in &env_keys {
        args.push("-e".to_string());
        args.push(key.clone());
    }
    args.push(reference);

    register_server(
        state,
        entry,
        "docker".to_string(),
        args,
        env_overrides,
        auto_start,
    )
    .await
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Marketplace servers are always installed to {data_dir}/mcp-servers/,
/// separate from config-loaded servers in [paths].servers.
fn resolve_servers_dir(state: &AppState) -> PathBuf {
    state.data_dir.join("mcp-servers")
}

/// Resolve the on-disk directory name a marketplace install uses under
/// `{data_dir}/mcp-servers/`. Falls back to `entry.id` when
/// `entry.directory` is empty so callers never hit the
/// `PathBuf::join("")` degeneracy (which collapses to the
/// `mcp-servers/` parent itself — flips the catalog's installed badge
/// for every empty-directory entry, and worse: lets uninstall
/// `remove_dir_all` walk the parent).
fn effective_install_dir(entry: &RegistryEntry) -> &str {
    let raw = if entry.directory.is_empty() {
        entry.id.as_str()
    } else {
        entry.directory.as_str()
    };
    // The install directory is a single path component under mcp-servers. A
    // catalog `directory` that carries the monorepo-relative path (e.g.
    // "servers/websearch") would otherwise be joined with the source `subdir`
    // (also "servers/websearch") at the extract / register / uninstall sites,
    // doubling the on-disk path to
    // mcp-servers/servers/<name>/servers/<name>/server.py — the launch then
    // fails because no server.py exists there (bug-399, Task #105). Collapse to
    // the final path component so a malformed multi-segment value stays flat;
    // well-formed ids (incl. dotted "memory.cpersona") pass through unchanged.
    raw.trim_matches('/')
        .rsplit(['/', '\\'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(raw)
}

/// Resolve the on-disk install directory for `entry` under `servers_dir`,
/// refusing any catalog `id`/`directory` that would escape `servers_dir`
/// (bug-430 / bug-432). The catalog is hub-served and can be stale or malicious;
/// `effective_install_dir` collapses multi-segment values, but a bare `..` (no
/// separator for `rsplit` to strip) would otherwise pass through and make
/// `servers_dir.join("..")` resolve to the parent data directory — turning the
/// pre-clone / pre-extract `remove_dir_all` into a wipe of app state. Mirrors the
/// containment intent of `uninstall_handler`'s bug-400 check, expressed as a
/// filesystem-independent single-component guard because the install target does
/// not exist yet (so `canonicalize` on it is unavailable here).
fn resolve_install_dir(
    servers_dir: &std::path::Path,
    entry: &RegistryEntry,
) -> anyhow::Result<PathBuf> {
    let component = effective_install_dir(entry);
    // The install dir must be exactly one normal path component. This rejects
    // `..` (ParentDir), `.` (CurDir), an absolute path (RootDir/Prefix — which
    // `join` would use to REPLACE `servers_dir` wholesale), any multi-segment
    // value, and the empty string; anything else could escape `servers_dir`.
    let mut components = std::path::Path::new(component).components();
    let single_normal = matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none();
    if !single_normal {
        anyhow::bail!(
            "Refusing to install '{}': install directory '{}' is not a single \
             path component under mcp-servers (path traversal blocked)",
            entry.id,
            component
        );
    }
    Ok(servers_dir.join(component))
}

/// Derive the on-disk install directory from a registered server's `args`
/// JSON (bug-392): the first arg that is an absolute path under
/// `servers_root` names the install directory as its first path component.
/// Returns `None` for unparseable args or paths outside the servers root
/// (e.g. dev-layout scripts).
fn install_dir_from_args(
    args_json: Option<&str>,
    servers_root: &std::path::Path,
) -> Option<String> {
    let args: Vec<String> = serde_json::from_str(args_json?).ok()?;
    args.iter().find_map(|a| {
        let rel = std::path::Path::new(a).strip_prefix(servers_root).ok()?;
        rel.components().next().and_then(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().to_string()),
            _ => None,
        })
    })
}

/// Locate an installable source tree for the shared `common` package
/// (bug-389/390). Two layouts exist on disk:
///
/// - nested git clone: `<clone>/servers/<name>` with `common` as a sibling
///   (`server_path.parent()/common`)
/// - legacy flat monorepo-tarball extraction: `<servers_dir>/common`
///
/// Only a tree carrying `pyproject.toml` (shipped with clotohub-servers
/// >= v0.1.8) can be `uv pip install`ed into the shared venv.
fn resolve_common_source(
    server_path: &std::path::Path,
    servers_dir: &std::path::Path,
) -> Option<std::path::PathBuf> {
    [
        server_path.parent().map(|p| p.join("common")),
        Some(servers_dir.join("common")),
    ]
    .into_iter()
    .flatten()
    .find(|p| p.join("pyproject.toml").is_file())
}

/// Validate that a destination path stays within the target directory (zip-slip prevention).
fn validate_dest_path(target_dir: &std::path::Path, dest: &std::path::Path) -> anyhow::Result<()> {
    // Normalize both paths to catch ".." traversal
    let normalized = dest
        .components()
        .fold(std::path::PathBuf::new(), |mut acc, c| {
            match c {
                std::path::Component::ParentDir => {
                    acc.pop();
                }
                std::path::Component::Normal(s) => acc.push(s),
                std::path::Component::RootDir => acc.push("/"),
                std::path::Component::Prefix(p) => acc.push(p.as_os_str()),
                std::path::Component::CurDir => {}
            }
            acc
        });
    let full = if dest.is_absolute() {
        normalized
    } else {
        target_dir.join(&normalized)
    };
    if !full.starts_with(target_dir) {
        anyhow::bail!(
            "Path traversal detected: '{}' escapes target directory",
            dest.display()
        );
    }
    Ok(())
}

/// Extract specific directories from a GitHub tarball.
/// GitHub tarballs have a prefix like `Cloto-dev-clotohub-servers-{sha}/`
/// (matching is suffix-based, so the repo name does not matter).
fn extract_selective(
    archive_path: &std::path::Path,
    target_dir: &std::path::Path,
    server_directory: &str,
    include_common: bool,
) -> anyhow::Result<()> {
    let file = std::fs::File::open(archive_path)?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);

    let server_suffix = format!("/servers/{server_directory}/");
    let common_suffix = "/servers/common/";

    for entry_result in archive.entries()? {
        let mut entry = entry_result?;
        let path = entry.path()?.to_path_buf();
        let path_str = path.to_string_lossy().to_string();

        // Check if this entry matches our target directories
        let relative = if path_str.contains(&server_suffix) {
            // Extract path after "servers/"
            path_str.find("/servers/").map(|i| &path_str[i + 1..]) // "servers/{directory}/..."
        } else if include_common && path_str.contains(common_suffix) {
            path_str.find("/servers/").map(|i| &path_str[i + 1..])
        } else {
            None
        };

        if let Some(relative) = relative {
            // Strip "servers/" prefix to get "{directory}/..."
            let dest = target_dir.join(relative.strip_prefix("servers/").unwrap_or(relative));
            validate_dest_path(target_dir, &dest)?;

            if entry.header().entry_type().is_dir() {
                std::fs::create_dir_all(&dest)?;
            } else {
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut out = std::fs::File::create(&dest)?;
                std::io::copy(&mut entry, &mut out)?;
            }
        }
    }

    Ok(())
}

/// Detect a single shared top-level directory across all tarball
/// entries. Returns `Some(prefix)` when every non-empty entry sits under
/// the same first path component (the GitHub `archive/<ref>.tar.gz`
/// convention), or `None` when entries are at the root or split across
/// multiple top-level dirs.
///
/// Real GitHub archives begin with pax metadata pseudo-entries
/// (`pax_global_header`) and an entry for the top-level directory
/// itself (`<repo>-<ref>/`) — both are tolerated: pax entries are
/// skipped outright, and a directory entry that *is* the top-level dir
/// names the prefix without disqualifying it. Only a top-level *file*
/// (or a second distinct top-level component) proves there is no shared
/// prefix.
fn detect_shared_prefix<R: std::io::Read>(
    archive: &mut tar::Archive<R>,
) -> anyhow::Result<Option<std::path::PathBuf>> {
    let mut prefix: Option<std::path::PathBuf> = None;
    let mut saw_nested = false;
    for entry_result in archive.entries()? {
        let entry = entry_result?;
        if is_pax_metadata(entry.header().entry_type()) {
            continue;
        }
        let path = entry.path()?.to_path_buf();
        let first = match path.components().next() {
            Some(c) => std::path::PathBuf::from(c.as_os_str()),
            None => continue,
        };
        let has_more = path.components().count() > 1;
        if !has_more && !entry.header().entry_type().is_dir() {
            // A top-level file can never be stripped.
            return Ok(None);
        }
        match &prefix {
            None => prefix = Some(first),
            Some(p) if *p != first => return Ok(None),
            _ => {}
        }
        if has_more {
            saw_nested = true;
        }
    }
    // A prefix is only strippable when something actually lives under it.
    Ok(if saw_nested { prefix } else { None })
}

/// True for tar pseudo-entries carrying pax metadata rather than file
/// content (`pax_global_header` and per-file extended headers). The
/// `tar` crate surfaces them as ordinary entries; treating them as
/// files corrupts both prefix detection and extraction output.
fn is_pax_metadata(entry_type: tar::EntryType) -> bool {
    matches!(
        entry_type,
        tar::EntryType::XGlobalHeader | tar::EntryType::XHeader
    )
}

/// Extract a `.tar.gz` into `target_dir`, stripping a single shared
/// top-level directory prefix when the archive uses one. Zip-slip safe
/// via [`validate_dest_path`].
fn extract_tarball_stripped(
    archive_path: &std::path::Path,
    target_dir: &std::path::Path,
) -> anyhow::Result<()> {
    // First pass: detect a shared prefix without writing any files.
    let strip_prefix = {
        let file = std::fs::File::open(archive_path)?;
        let decoder = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(decoder);
        detect_shared_prefix(&mut archive)?
    };

    // Second pass: extract.
    let file = std::fs::File::open(archive_path)?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);

    for entry_result in archive.entries()? {
        let mut entry = entry_result?;
        if is_pax_metadata(entry.header().entry_type()) {
            continue;
        }
        let path = entry.path()?.to_path_buf();

        let relative = match &strip_prefix {
            Some(prefix) => match path.strip_prefix(prefix) {
                Ok(p) if p.as_os_str().is_empty() => continue,
                Ok(p) => p.to_path_buf(),
                Err(_) => continue,
            },
            None => path,
        };

        if relative.as_os_str().is_empty() {
            continue;
        }

        let dest = target_dir.join(&relative);
        validate_dest_path(target_dir, &dest)?;

        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&dest)?;
        } else {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out = std::fs::File::create(&dest)?;
            std::io::copy(&mut entry, &mut out)?;
        }
    }

    Ok(())
}

/// Extract one subdirectory tree (plus, optionally, its sibling
/// `common/` package) from a tarball, preserving repo-relative paths
/// under `target_dir` after stripping a single shared top-level prefix
/// (GitHub-archive-style). Used by the `raw_url` + `subdir` install
/// path (Goal #110, mgp-sdk v0.3.0): the resulting layout
/// (`{target_dir}/{subdir}/…` with `common` as the subdir's sibling)
/// matches a nested git clone, so `resolve_common_source` and the
/// uninstall directory candidates work unchanged. Zip-slip safe via
/// [`validate_dest_path`].
fn extract_subdir_selective(
    archive_path: &std::path::Path,
    target_dir: &std::path::Path,
    subdir: &str,
    include_common: bool,
) -> anyhow::Result<()> {
    // First pass: detect a shared prefix without writing any files.
    let strip_prefix = {
        let file = std::fs::File::open(archive_path)?;
        let decoder = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(decoder);
        detect_shared_prefix(&mut archive)?
    };

    let subdir_root = std::path::PathBuf::from(subdir);
    // Sibling `common/` package: `servers/cscheduler` → `servers/common`,
    // a root-level `cscheduler` → `common`.
    let common_root = match subdir_root.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.join("common"),
        _ => std::path::PathBuf::from("common"),
    };

    let file = std::fs::File::open(archive_path)?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);

    let mut extracted_any = false;
    for entry_result in archive.entries()? {
        let mut entry = entry_result?;
        if is_pax_metadata(entry.header().entry_type()) {
            continue;
        }
        let path = entry.path()?.to_path_buf();

        let relative = match &strip_prefix {
            Some(prefix) => match path.strip_prefix(prefix) {
                Ok(p) if p.as_os_str().is_empty() => continue,
                Ok(p) => p.to_path_buf(),
                Err(_) => continue,
            },
            None => path,
        };

        let wanted = relative.starts_with(&subdir_root)
            || (include_common && relative.starts_with(&common_root));
        if !wanted {
            continue;
        }

        let dest = target_dir.join(&relative);
        validate_dest_path(target_dir, &dest)?;

        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&dest)?;
        } else {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out = std::fs::File::create(&dest)?;
            std::io::copy(&mut entry, &mut out)?;
            if relative.starts_with(&subdir_root) {
                extracted_any = true;
            }
        }
    }

    if !extracted_any {
        anyhow::bail!("tarball contains no files under subdir '{subdir}'");
    }
    Ok(())
}

/// Extract multiple server directories from a GitHub tarball in a single pass.
fn extract_batch(
    archive_path: &std::path::Path,
    target_dir: &std::path::Path,
    server_directories: &[String],
    include_common: bool,
) -> anyhow::Result<()> {
    let file = std::fs::File::open(archive_path)?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);

    let suffixes: Vec<String> = server_directories
        .iter()
        .map(|d| format!("/servers/{d}/"))
        .collect();
    let common_suffix = "/servers/common/";

    for entry_result in archive.entries()? {
        let mut entry = entry_result?;
        let path = entry.path()?.to_path_buf();
        let path_str = path.to_string_lossy().to_string();

        let relative = suffixes
            .iter()
            .find(|suffix| path_str.contains(suffix.as_str()))
            .and_then(|_| path_str.find("/servers/").map(|i| &path_str[i + 1..]))
            .or_else(|| {
                if include_common && path_str.contains(common_suffix) {
                    path_str.find("/servers/").map(|i| &path_str[i + 1..])
                } else {
                    None
                }
            });

        if let Some(relative) = relative {
            let dest = target_dir.join(relative.strip_prefix("servers/").unwrap_or(relative));
            validate_dest_path(target_dir, &dest)?;

            if entry.header().entry_type().is_dir() {
                std::fs::create_dir_all(&dest)?;
            } else {
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut out = std::fs::File::create(&dest)?;
                std::io::copy(&mut entry, &mut out)?;
            }
        }
    }

    Ok(())
}

// ── Batch install orchestration ─────────────────────────────────────

#[allow(clippy::too_many_lines)]
async fn run_batch_install(
    state: &AppState,
    entries: &[RegistryEntry],
    auto_start: bool,
) -> anyhow::Result<()> {
    let tx = &state.setup_progress_tx;
    let has_python_servers = entries.iter().any(|e| e.runtime != "rust");
    let has_rust_servers = entries.iter().any(|e| e.runtime == "rust");

    // Step 1: Ensure uv is available (auto-download if missing)
    if has_python_servers {
        emit(
            tx,
            SetupProgressEvent::StepStart {
                step: "check_uv".into(),
                description: "Setting up uv package manager".into(),
            },
        );
        let uv = crate::managers::mcp_venv::uv_bin(&state.data_dir);
        if !uv.exists() {
            info!("Batch install: uv not found, downloading...");
            if let Err(e) = super::setup::download_uv(tx, &state.data_dir).await {
                emit(
                    tx,
                    SetupProgressEvent::StepError {
                        step: "check_uv".into(),
                        error: format!("Failed to download uv: {e}"),
                        recoverable: true,
                    },
                );
                return Ok(());
            }
        }
        emit(
            tx,
            SetupProgressEvent::StepComplete {
                step: "check_uv".into(),
            },
        );
    }
    if has_rust_servers {
        emit(
            tx,
            SetupProgressEvent::StepStart {
                step: "check_cargo".into(),
                description: "Checking Rust toolchain".into(),
            },
        );
        let (cargo_available, _) = detect_cargo();
        if !cargo_available {
            emit(
                tx,
                SetupProgressEvent::StepError {
                    step: "check_cargo".into(),
                    error: "Rust toolchain (cargo) is required for some servers. Install via https://rustup.rs/".into(),
                    recoverable: false,
                },
            );
            return Ok(());
        }
        emit(
            tx,
            SetupProgressEvent::StepComplete {
                step: "check_cargo".into(),
            },
        );
    }

    // ── Step 0: Clean stale state from previous installations ──
    let servers_dir = state.data_dir.join("mcp-servers");
    let tmp_dir = state.data_dir.join("tmp");

    // Clean stale tarball
    if tmp_dir.join("cloto-mcp-servers-latest.tar.gz").exists() {
        info!("Cleaning stale tarball from previous install");
        let _ = tokio::fs::remove_file(tmp_dir.join("cloto-mcp-servers-latest.tar.gz")).await;
    }

    // Check if installed servers are outdated (version mismatch with current binary)
    let setup_json = state.data_dir.join("setup-complete.json");
    let needs_clean = if setup_json.exists() {
        if let Ok(content) = tokio::fs::read_to_string(&setup_json).await {
            serde_json::from_str::<serde_json::Value>(&content)
                .ok()
                .and_then(|v| v.get("version")?.as_str().map(String::from))
                .is_some_and(|v| v != env!("CARGO_PKG_VERSION"))
        } else {
            false
        }
    } else {
        false
    };

    if needs_clean {
        emit(
            tx,
            SetupProgressEvent::StepStart {
                step: "cleanup".into(),
                description: "Cleaning previous installation".into(),
            },
        );

        // Remove old venv entirely
        let venv_dir = servers_dir.join(".venv");
        if venv_dir.exists() {
            info!("Removing stale venv (version mismatch)");
            let _ = tokio::fs::remove_dir_all(&venv_dir).await;
        }

        // Remove old server directories (keep mcp-servers/ itself)
        if servers_dir.exists() {
            if let Ok(mut dir_entries) = tokio::fs::read_dir(&servers_dir).await {
                while let Ok(Some(entry)) = dir_entries.next_entry().await {
                    let path = entry.path();
                    if path.is_dir() && path.file_name().is_some_and(|n| n != ".venv") {
                        let _ = tokio::fs::remove_dir_all(&path).await;
                    }
                }
            }
        }

        // Remove stale setup-complete.json
        let _ = tokio::fs::remove_file(&setup_json).await;

        emit(
            tx,
            SetupProgressEvent::StepComplete {
                step: "cleanup".into(),
            },
        );
    }

    // Step 2: Download tarball (once for all servers)
    emit(
        tx,
        SetupProgressEvent::StepStart {
            step: "download".into(),
            description: format!("Downloading {} servers", entries.len()),
        },
    );

    let tmp_dir = state.data_dir.join("tmp");
    tokio::fs::create_dir_all(&tmp_dir).await?;
    let archive_path = tmp_dir.join("cloto-mcp-servers-latest.tar.gz");

    let tarball_url = tarball_url("master");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(TARBALL_DOWNLOAD_TIMEOUT_SECS))
        .build()?;
    let resp = client
        .get(tarball_url)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "ClotoCore")
        .send()
        .await?;

    if !resp.status().is_success() {
        emit(
            tx,
            SetupProgressEvent::StepError {
                step: "download".into(),
                error: format!("GitHub API returned HTTP {}", resp.status()),
                recoverable: true,
            },
        );
        return Ok(());
    }

    let total = resp.content_length();
    let mut stream = resp.bytes_stream();
    let mut file = tokio::fs::File::create(&archive_path).await?;
    let mut downloaded: u64 = 0;

    use futures::StreamExt;
    use tokio::io::AsyncWriteExt;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        if let Some(total) = total {
            let progress = (downloaded as f32 / total as f32).min(1.0);
            let mb_done = downloaded as f64 / 1_048_576.0;
            let mb_total = total as f64 / 1_048_576.0;
            emit(
                tx,
                SetupProgressEvent::StepProgress {
                    step: "download".into(),
                    progress,
                    detail: format!("{mb_done:.1} / {mb_total:.1} MB"),
                },
            );
        }
    }
    file.flush().await?;
    emit(
        tx,
        SetupProgressEvent::StepComplete {
            step: "download".into(),
        },
    );

    // Step 3: Batch extraction
    emit(
        tx,
        SetupProgressEvent::StepStart {
            step: "extract".into(),
            description: format!("Extracting {} servers", entries.len()),
        },
    );

    let servers_dir = resolve_servers_dir(state);
    let directories: Vec<String> = entries.iter().map(|e| e.directory.clone()).collect();
    let needs_common = entries
        .iter()
        .any(|e| e.runtime != "rust" && e.dependencies.contains(&"common".to_string()));

    let archive_clone = archive_path.clone();
    let servers_dir_clone = servers_dir.clone();
    tokio::task::spawn_blocking(move || {
        extract_batch(
            &archive_clone,
            &servers_dir_clone,
            &directories,
            needs_common,
        )
    })
    .await??;
    emit(
        tx,
        SetupProgressEvent::StepComplete {
            step: "extract".into(),
        },
    );

    // Step 4: Ensure venv + install dependencies
    emit(
        tx,
        SetupProgressEvent::StepStart {
            step: "install_deps".into(),
            description: "Installing dependencies".into(),
        },
    );

    // Debug log: write diagnostic info to a file at each step
    let diag_log = state.data_dir.join("install-debug.log");
    let diag = |msg: &str| {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&diag_log)
        {
            let _ = writeln!(f, "[{}] {msg}", chrono::Local::now().format("%H:%M:%S%.3f"));
        }
    };

    diag("install_deps started");

    let venv_dir =
        crate::managers::mcp_venv::resolve_venv_dir().unwrap_or_else(|| servers_dir.join(".venv"));
    diag(&format!("venv_dir={}", venv_dir.display()));

    let mut python_entries: Vec<&RegistryEntry> = Vec::new();
    let mut rust_entries: Vec<&RegistryEntry> = Vec::new();
    for entry in entries {
        if entry.runtime == "rust" {
            rust_entries.push(entry);
        } else {
            python_entries.push(entry);
        }
    }

    diag(&format!(
        "python_entries={}, rust_entries={}, servers_dir={}",
        python_entries.len(),
        rust_entries.len(),
        servers_dir.display()
    ));

    if !python_entries.is_empty() {
        diag("Phase 1: Python servers — starting");

        // Pre-cleanup: remove stale/corrupted venv
        if crate::managers::mcp_venv::is_venv_stale(&venv_dir) && venv_dir.exists() {
            diag("Removing stale venv");
            info!("Removing stale venv (Python version mismatch)");
            let _ = tokio::fs::remove_dir_all(&venv_dir).await;
        }
        if venv_dir.exists() && !venv_dir.join("pyvenv.cfg").exists() {
            diag("Removing corrupted venv (no pyvenv.cfg)");
            warn!("Removing corrupted venv directory (no pyvenv.cfg)");
            let _ = tokio::fs::remove_dir_all(&venv_dir).await;
        }

        let uv = crate::managers::mcp_venv::uv_bin(&state.data_dir);
        let uv_str = uv.to_string_lossy().to_string();
        let target_python = crate::managers::mcp_venv::TARGET_PYTHON;
        let venv_python = crate::managers::mcp_venv::venv_python(&venv_dir);
        let venv_python_str = venv_python.to_string_lossy().to_string();
        let need_venv = !venv_dir.join("pyvenv.cfg").exists();
        let common_path = if needs_common {
            let p = servers_dir.join("common");
            if p.join("pyproject.toml").exists() {
                Some(p)
            } else {
                None
            }
        } else {
            None
        };

        diag(&format!(
            "uv={}, need_venv={}, needs_common={}, common_path={:?}",
            uv_str, need_venv, needs_common, common_path
        ));

        // Prepare log file
        let logs_dir = state.data_dir.join("logs");
        let _ = tokio::fs::create_dir_all(&logs_dir).await;
        let log_file = logs_dir.join("install.log");

        // Create venv if needed (direct uv spawn)
        if need_venv {
            diag("Creating venv...");
            emit(
                tx,
                SetupProgressEvent::StepProgress {
                    step: "install_deps".into(),
                    progress: 0.05,
                    detail: "Creating Python virtual environment".into(),
                },
            );
            let venv_path_str = venv_dir.to_string_lossy().to_string();
            diag(&format!(
                "uv venv --python {} {}",
                target_python, venv_path_str
            ));
            let mut cmd = tokio::process::Command::new(&uv_str);
            cmd.args(["venv", "--python", target_python, &venv_path_str])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            #[cfg(windows)]
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
            let result = tokio::time::timeout(
                Duration::from_secs(CHILD_PROCESS_TIMEOUT_SECS),
                cmd.status(),
            )
            .await;
            match &result {
                Ok(Ok(status)) if status.success() => diag("venv created OK"),
                Ok(Ok(status)) => diag(&format!("venv FAILED exit={:?}", status.code())),
                Ok(Err(e)) => diag(&format!("venv spawn ERROR: {e}")),
                Err(_) => diag("venv TIMEOUT (120s)"),
            }
            match result {
                Ok(Ok(status)) if status.success() => {
                    info!(
                        "Created Python {} venv at {}",
                        target_python,
                        venv_dir.display()
                    );
                }
                Ok(Ok(status)) => {
                    emit(
                        tx,
                        SetupProgressEvent::StepError {
                            step: "install_deps".into(),
                            error: format!(
                                "Failed to create venv (exit code: {:?})",
                                status.code()
                            ),
                            recoverable: true,
                        },
                    );
                    return Ok(());
                }
                Ok(Err(e)) => {
                    emit(
                        tx,
                        SetupProgressEvent::StepError {
                            step: "install_deps".into(),
                            error: format!("Failed to run uv for venv creation: {e}"),
                            recoverable: true,
                        },
                    );
                    return Ok(());
                }
                Err(_) => {
                    emit(
                        tx,
                        SetupProgressEvent::StepError {
                            step: "install_deps".into(),
                            error: "uv venv creation timed out (120s)".into(),
                            recoverable: true,
                        },
                    );
                    return Ok(());
                }
            }
        } else {
            diag("venv already exists, skipping creation");
        }

        // Install common if needed (direct uv spawn)
        if let Some(ref cp) = common_path {
            emit(
                tx,
                SetupProgressEvent::ServerInstall {
                    server_name: "common".into(),
                    status: "installing".into(),
                },
            );
            let mut cmd = tokio::process::Command::new(&uv_str);
            cmd.args([
                "pip",
                "install",
                "--no-progress",
                "--python",
                &venv_python_str,
                &cp.to_string_lossy(),
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true); // bug-366: reap on install-task abort
            #[cfg(windows)]
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
            let result = tokio::time::timeout(
                Duration::from_secs(CHILD_PROCESS_TIMEOUT_SECS),
                cmd.status(),
            )
            .await;
            match result {
                Ok(Ok(status)) if status.success() => {
                    emit(
                        tx,
                        SetupProgressEvent::ServerInstall {
                            server_name: "common".into(),
                            status: "installed".into(),
                        },
                    );
                }
                Err(_) => warn!("Common module install timed out (120s)"),
                _ => warn!("Failed to install common dependency"),
            }
        }

        diag("Starting per-server installs");
        // Install each Python server individually via spawn_uv_streaming
        for entry in &python_entries {
            emit(
                tx,
                SetupProgressEvent::ServerInstall {
                    server_name: entry.name.clone(),
                    status: "installing".into(),
                },
            );
            let server_path = servers_dir
                .join(&entry.directory)
                .to_string_lossy()
                .to_string();

            diag(&format!(
                "Installing server: {} (path: {})",
                entry.name, server_path
            ));
            match super::setup::spawn_uv_streaming(
                tx,
                &uv_str,
                &venv_python_str,
                &entry.name,
                &server_path,
                Some(&log_file),
            )
            .await
            {
                Ok(()) => {
                    diag(&format!("  {} → OK", entry.name));
                    emit(
                        tx,
                        SetupProgressEvent::ServerInstall {
                            server_name: entry.name.clone(),
                            status: "installed".into(),
                        },
                    );
                }
                Err(detail) => {
                    diag(&format!("  {} → FAILED: {}", entry.name, detail));
                    warn!("Failed to install {}: {}", entry.name, detail);
                    emit(
                        tx,
                        SetupProgressEvent::ServerInstall {
                            server_name: entry.name.clone(),
                            status: "failed".into(),
                        },
                    );
                    // Continue to next server — error isolation
                }
            }
        }
    }

    // ── Phase 2: Build Rust servers individually ──
    for entry in &rust_entries {
        let server_path = servers_dir.join(&entry.directory);
        emit(
            tx,
            SetupProgressEvent::ServerInstall {
                server_name: entry.name.clone(),
                status: "installing".into(),
            },
        );

        let cargo_toml = server_path.join("Cargo.toml");
        if cargo_toml.exists() {
            let content = std::fs::read_to_string(&cargo_toml).unwrap_or_default();
            if !content.contains("[workspace]") {
                std::fs::write(&cargo_toml, format!("{content}\n[workspace]\n"))
                    .unwrap_or_else(|e| warn!("Failed to patch Cargo.toml: {e}"));
            }
        }

        if !matches!(
            cargo_build_server(tx, &server_path, &entry.name).await,
            Ok(true)
        ) {
            emit(
                tx,
                SetupProgressEvent::ServerInstall {
                    server_name: entry.name.clone(),
                    status: "failed".into(),
                },
            );
        }
    }

    // ── Phase 3: Register all servers with MCP manager ──
    let all_entries: Vec<&RegistryEntry> = python_entries
        .iter()
        .chain(rust_entries.iter())
        .copied()
        .collect();

    for entry in &all_entries {
        let server_path = servers_dir.join(&entry.directory);
        let is_rust = entry.runtime == "rust";

        let (command, args) = if is_rust {
            let bin_path = rust_binary_path(&server_path, entry);
            (bin_path.to_string_lossy().to_string(), vec![])
        } else {
            let server_script = server_path.join("server.py").to_string_lossy().to_string();
            // Use bare "python" — resolved to venv python at spawn time
            // by resolve_python_command() in mcp_transport.
            ("python".to_string(), vec![server_script])
        };

        let mut env_map: HashMap<String, String> = HashMap::new();
        for var in &entry.env_vars {
            if let Some(default) = &var.default {
                env_map.insert(var.name.clone(), default.clone());
            }
        }

        // bug-394: Ed25519 verify + keyless integrity check + local
        // re-seal (see local_seal_for_install). A tamper-suspect failure
        // skips registration of this entry rather than aborting the
        // whole batch.
        let hub_key = match entry_signature_kid(entry) {
            Some(kid) => hub_signing_key(state, &kid).await,
            None => None,
        };
        let seal =
            match local_seal_for_install(&state.data_dir, entry, &command, &args, hub_key.as_ref())
            {
                Ok(seal) => seal,
                Err(e) => {
                    warn!("Batch: {} skipped — {e}", entry.id);
                    continue;
                }
            };
        let mgp = Some(crate::managers::mcp_mgp::MgpServerConfig {
            trust_level: Some(entry.trust_level.clone()),
        });
        let connected = match state
            .mcp_manager
            .add_server(
                entry.id.clone(),
                command,
                args,
                None,
                Some(entry.description.clone()),
                mgp,
                seal,
                env_map,
            )
            .await
        {
            Ok(tools) => {
                info!("Batch: {} connected ({} tools)", entry.id, tools.len());
                true
            }
            Err(e) => {
                warn!("Batch: {} registered but failed to connect: {e}", entry.id);
                false
            }
        };

        if let Err(e) = crate::db::mcp::set_marketplace_fields(
            &state.pool,
            &entry.id,
            &entry.version,
            &entry.id,
            Some(&entry.trust_level),
        )
        .await
        {
            warn!("Failed to set marketplace fields for {}: {e}", entry.id);
        }

        if !auto_start {
            let _ = state.mcp_manager.stop_server(&entry.id).await;
        }

        emit(
            tx,
            SetupProgressEvent::ServerInstall {
                server_name: entry.name.clone(),
                status: if connected { "installed" } else { "registered" }.into(),
            },
        );
    }

    emit(
        tx,
        SetupProgressEvent::StepComplete {
            step: "install_deps".into(),
        },
    );

    // Write setup-complete.json (for ensure_mcp_venv gate)
    let complete = serde_json::json!({
        "completed_at": chrono::Utc::now().to_rfc3339(),
        "version": env!("CARGO_PKG_VERSION"),
        "mcp_servers_version": env!("CARGO_PKG_VERSION"),
        "server_count": entries.len(),
    });
    // Atomic write: temp file + rename to prevent corruption (bug-362)
    let setup_json = state.data_dir.join("setup-complete.json");
    let setup_tmp = state.data_dir.join("setup-complete.json.tmp");
    if let Ok(json) = serde_json::to_string_pretty(&complete) {
        if let Err(e) = tokio::fs::write(&setup_tmp, &json).await {
            warn!("Failed to write setup-complete.json.tmp: {e}");
        } else if let Err(e) = tokio::fs::rename(&setup_tmp, &setup_json).await {
            warn!("Failed to rename setup-complete.json.tmp: {e}");
        }
    }

    // Cleanup
    if let Err(e) = tokio::fs::remove_file(&archive_path).await {
        warn!("Failed to cleanup archive {}: {e}", archive_path.display());
    }

    emit(tx, SetupProgressEvent::Complete);
    info!("Batch install complete: {} server(s)", entries.len());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a gzip tarball at `path` from `(entry_path, bytes)` pairs,
    /// mirroring a *real* GitHub archive: a `pax_global_header`
    /// pseudo-entry and an entry for the top-level directory itself
    /// precede the file entries (shared top-level prefix included by
    /// the caller in each entry path).
    fn write_tarball(path: &std::path::Path, files: &[(&str, &[u8])]) {
        let mut tar_buf = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_buf);

            let payload: &[u8] = b"52 comment=0123456789abcdef0123456789abcdef01234567\n";
            let mut pax = tar::Header::new_gnu();
            pax.set_entry_type(tar::EntryType::XGlobalHeader);
            pax.set_size(payload.len() as u64);
            pax.set_mode(0o644);
            builder
                .append_data(&mut pax, "pax_global_header", payload)
                .unwrap();

            let mut dir = tar::Header::new_gnu();
            dir.set_entry_type(tar::EntryType::Directory);
            dir.set_size(0);
            dir.set_mode(0o755);
            builder.append_data(&mut dir, "repo-v0/", &[][..]).unwrap();

            for (name, data) in files {
                let mut header = tar::Header::new_gnu();
                header.set_size(data.len() as u64);
                header.set_mode(0o644);
                builder.append_data(&mut header, name, *data).unwrap();
            }
            builder.finish().unwrap();
        }
        use std::io::Write;
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&tar_buf).unwrap();
        std::fs::write(path, gz.finish().unwrap()).unwrap();
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "clotocore-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const MONOREPO_FILES: &[(&str, &[u8])] = &[
        ("repo-v0/README.md", b"readme"),
        ("repo-v0/servers/demo/server.py", b"print('demo')"),
        ("repo-v0/servers/demo/pyproject.toml", b"[project]"),
        ("repo-v0/servers/common/pyproject.toml", b"[project]"),
        ("repo-v0/servers/common/common/__init__.py", b""),
        ("repo-v0/servers/other/server.py", b"print('other')"),
    ];

    #[test]
    fn extract_subdir_selective_extracts_server_and_sibling_common() {
        let work = temp_dir("subdir-common");
        let archive = work.join("repo.tar.gz");
        write_tarball(&archive, MONOREPO_FILES);
        let target = work.join("out");
        std::fs::create_dir_all(&target).unwrap();

        extract_subdir_selective(&archive, &target, "servers/demo", true).unwrap();

        // Nested-git-clone layout: repo-relative paths preserved.
        assert!(target.join("servers/demo/server.py").is_file());
        assert!(target.join("servers/demo/pyproject.toml").is_file());
        assert!(target.join("servers/common/pyproject.toml").is_file());
        // Unrelated trees stay out.
        assert!(!target.join("servers/other").exists());
        assert!(!target.join("README.md").exists());
        // `resolve_common_source` finds common as the server's sibling.
        let server_path = target.join("servers/demo");
        let found = resolve_common_source(&server_path, &target);
        assert_eq!(found, Some(target.join("servers/common")));
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn extract_subdir_selective_skips_common_when_not_needed() {
        let work = temp_dir("subdir-nocommon");
        let archive = work.join("repo.tar.gz");
        write_tarball(&archive, MONOREPO_FILES);
        let target = work.join("out");
        std::fs::create_dir_all(&target).unwrap();

        extract_subdir_selective(&archive, &target, "servers/demo", false).unwrap();

        assert!(target.join("servers/demo/server.py").is_file());
        assert!(!target.join("servers/common").exists());
        let _ = std::fs::remove_dir_all(&work);
    }

    /// Regression for the Goal #110 E2E failure: real GitHub archives
    /// open with `pax_global_header` and the top-level directory entry,
    /// both of which made `detect_shared_prefix` give up (`None`) — so
    /// stripped extraction kept the `<repo>-<ref>/` wrapper and subdir
    /// extraction matched nothing.
    #[test]
    fn extract_tarball_stripped_strips_prefix_on_github_style_archive() {
        let work = temp_dir("stripped-github");
        let archive = work.join("repo.tar.gz");
        write_tarball(&archive, MONOREPO_FILES);
        let target = work.join("out");
        std::fs::create_dir_all(&target).unwrap();

        extract_tarball_stripped(&archive, &target).unwrap();

        assert!(target.join("servers/demo/server.py").is_file());
        assert!(
            !target.join("repo-v0").exists(),
            "the shared top-level prefix must be stripped"
        );
        assert!(
            !target.join("pax_global_header").exists(),
            "pax metadata must not be extracted as a file"
        );
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn extract_subdir_selective_errors_when_subdir_absent() {
        let work = temp_dir("subdir-missing");
        let archive = work.join("repo.tar.gz");
        write_tarball(&archive, MONOREPO_FILES);
        let target = work.join("out");
        std::fs::create_dir_all(&target).unwrap();

        let err = extract_subdir_selective(&archive, &target, "servers/nope", true);
        assert!(err.is_err(), "absent subdir must be a hard error");
        let _ = std::fs::remove_dir_all(&work);
    }

    fn entry(id: &str, directory: &str) -> RegistryEntry {
        RegistryEntry {
            id: id.into(),
            name: "test".into(),
            description: String::new(),
            category: "test".into(),
            version: "0.0.0".into(),
            directory: directory.into(),
            dependencies: vec![],
            env_vars: vec![],
            optional_env_vars: vec![],
            tags: vec![],
            trust_level: "standard".into(),
            auto_restart: false,
            icon: None,
            runtime: "python".into(),
            bin_name: None,
            changelog: None,
            seal: None,
            entry_point_sha256: None,
            signature_payload: None,
            install: None,
            provider: None,
        }
    }

    // ── local_seal_for_install (bug-394, D2 bifurcation) ──

    fn sha256_hex(data: &[u8]) -> String {
        use sha2::Digest;
        hex::encode(sha2::Sha256::digest(data))
    }

    /// Build a hub-side identity + a correctly signed entry for `script_body`.
    fn signed_entry(
        id: &str,
        script_body: &[u8],
    ) -> (
        RegistryEntry,
        mgp_seal::ed25519::PublicKey,
        mgp_seal::ed25519::KeyId,
    ) {
        let (sk, pk) = mgp_seal::ed25519::generate_keypair(&mut rand::rngs::OsRng);
        let kid = mgp_seal::ed25519::KeyId::new("clotohub-master-test").unwrap();
        let mut e = entry(id, id);
        let hash = sha256_hex(script_body);
        let canonical = mgp_seal::canonical_message(&e.id, &e.version, &hash);
        let sig = mgp_seal::ed25519::sign(&sk, &kid, &canonical);
        e.entry_point_sha256 = Some(hash);
        e.signature_payload = Some(serde_json::json!({
            "hmac": "ab".repeat(32),
            "ed25519": { "sig": sig.to_base64(), "key_id": kid.as_str() }
        }));
        (e, pk, kid)
    }

    #[test]
    fn local_seal_skips_when_catalog_has_no_hash() {
        let work = temp_dir("seal-nohash");
        let e = entry("demo", "demo");
        let got = local_seal_for_install(&work, &e, "python", &["x.py".into()], None).unwrap();
        assert_eq!(got, None, "no catalog hash -> register unsealed");
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn local_seal_unsealed_when_hash_matches_but_unsigned() {
        // Hash-only match no longer grants the declared tier: without an
        // Ed25519 signature the trust anchor would be the transport, which
        // is exactly what the proper fix retires. Register unsealed.
        let work = temp_dir("seal-unsigned");
        let script = work.join("server.py");
        std::fs::write(&script, b"print('demo')").unwrap();
        let mut e = entry("demo", "demo");
        e.entry_point_sha256 = Some(sha256_hex(b"print('demo')"));

        let got = local_seal_for_install(
            &work,
            &e,
            "python",
            &[script.to_string_lossy().to_string()],
            None,
        )
        .unwrap();
        assert_eq!(got, None, "unsigned entry -> register unsealed");
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn local_seal_blocks_on_hash_mismatch() {
        let work = temp_dir("seal-mismatch");
        let script = work.join("server.py");
        std::fs::write(&script, b"print('tampered')").unwrap();
        let mut e = entry("demo", "demo");
        e.entry_point_sha256 = Some(sha256_hex(b"print('demo')"));

        let err = local_seal_for_install(
            &work,
            &e,
            "python",
            &[script.to_string_lossy().to_string()],
            None,
        );
        assert!(err.is_err(), "hash mismatch must hard-error");
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn local_seal_minted_when_signature_verifies() {
        let work = temp_dir("seal-ed25519-ok");
        let body = b"print('demo')";
        let script = work.join("server.py");
        std::fs::write(&script, body).unwrap();
        let (e, pk, _kid) = signed_entry("demo", body);

        let got = local_seal_for_install(
            &work,
            &e,
            "python",
            &[script.to_string_lossy().to_string()],
            Some(&pk),
        )
        .unwrap()
        .expect("verified signature + matching hash must mint a local seal");

        // The minted seal verifies under the LOCAL key/protocol — exactly
        // what the spawn-time check will recompute.
        let key = mgp_seal::load_or_generate_seal_key(&work).unwrap();
        assert!(mgp_seal::verify_seal(&script, &got, &key).unwrap());
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn local_seal_blocks_on_invalid_signature() {
        // Resolvable key + well-formed signature that does not verify =
        // the signed identity does not match the served entry. Tamper
        // suspect: hard-block.
        let work = temp_dir("seal-ed25519-bad");
        let body = b"print('demo')";
        let script = work.join("server.py");
        std::fs::write(&script, body).unwrap();
        let (mut e, pk, _kid) = signed_entry("demo", body);
        e.version = "9.9.9".into(); // breaks the signed canonical message

        let err = local_seal_for_install(
            &work,
            &e,
            "python",
            &[script.to_string_lossy().to_string()],
            Some(&pk),
        );
        assert!(err.is_err(), "invalid signature must hard-block");
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn local_seal_unsealed_when_jwks_unavailable() {
        // Signature present but no key to check it with (hub down, kid
        // unknown after rotation). Benign: register unsealed, never block.
        let work = temp_dir("seal-ed25519-nokey");
        let body = b"print('demo')";
        let script = work.join("server.py");
        std::fs::write(&script, body).unwrap();
        let (e, _pk, _kid) = signed_entry("demo", body);

        let got = local_seal_for_install(
            &work,
            &e,
            "python",
            &[script.to_string_lossy().to_string()],
            None,
        )
        .unwrap();
        assert_eq!(got, None, "jwks unavailable -> register unsealed");
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn local_seal_unsealed_on_malformed_signature_block() {
        // Undecodable signature material is indistinguishable from catalog
        // corruption — benign untrusted, not a tamper verdict.
        let work = temp_dir("seal-ed25519-malformed");
        let body = b"print('demo')";
        let script = work.join("server.py");
        std::fs::write(&script, body).unwrap();
        let (mut e, pk, _kid) = signed_entry("demo", body);
        e.signature_payload = Some(serde_json::json!({
            "ed25519": { "sig": "!!!not-base64!!!", "key_id": "clotohub-master-test" }
        }));

        let got = local_seal_for_install(
            &work,
            &e,
            "python",
            &[script.to_string_lossy().to_string()],
            Some(&pk),
        )
        .unwrap();
        assert_eq!(got, None, "malformed signature -> register unsealed");
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn local_seal_mismatch_beats_valid_signature() {
        // Even with a valid signature, delivered bytes that do not hash to
        // the signed entry_point_sha256 are the strongest tamper signal.
        let work = temp_dir("seal-ed25519-swap");
        let script = work.join("server.py");
        std::fs::write(&script, b"print('swapped')").unwrap();
        let (e, pk, _kid) = signed_entry("demo", b"print('demo')");

        let err = local_seal_for_install(
            &work,
            &e,
            "python",
            &[script.to_string_lossy().to_string()],
            Some(&pk),
        );
        assert!(
            err.is_err(),
            "hash mismatch must hard-block even when signed"
        );
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn entry_signature_kid_extracts_from_payload() {
        let (e, _pk, kid) = signed_entry("demo", b"x");
        assert_eq!(entry_signature_kid(&e).as_deref(), Some(kid.as_str()));
        assert_eq!(entry_signature_kid(&entry("demo", "demo")), None);
    }

    // ── install_dir_from_args (bug-392) ──

    #[test]
    fn install_dir_derived_from_first_path_under_root() {
        let root = std::path::Path::new("/data/mcp-servers");
        let args = r#"["/data/mcp-servers/deepseek/server.py"]"#;
        assert_eq!(
            install_dir_from_args(Some(args), root),
            Some("deepseek".to_string())
        );
    }

    #[test]
    fn install_dir_skips_flags_and_outside_paths() {
        let root = std::path::Path::new("/data/mcp-servers");
        let args = r#"["-u", "/opt/elsewhere/server.py", "/data/mcp-servers/cscheduler/servers/cscheduler/server.py"]"#;
        assert_eq!(
            install_dir_from_args(Some(args), root),
            Some("cscheduler".to_string())
        );
    }

    #[test]
    fn install_dir_none_for_missing_or_invalid_args() {
        let root = std::path::Path::new("/data/mcp-servers");
        assert_eq!(install_dir_from_args(None, root), None);
        assert_eq!(install_dir_from_args(Some("not json"), root), None);
        assert_eq!(
            install_dir_from_args(Some(r#"["/opt/elsewhere/server.py"]"#), root),
            None
        );
    }

    // ── resolve_common_source (bug-389/390) ──

    #[test]
    fn common_source_prefers_nested_clone_sibling() {
        let tmp = tempfile::tempdir().unwrap();
        let servers_dir = tmp.path().join("mcp-servers");
        // nested git clone: <servers_dir>/<dir>/servers/<name> + sibling common
        let server_path = servers_dir.join("cscheduler/servers/cscheduler");
        let nested_common = servers_dir.join("cscheduler/servers/common");
        let flat_common = servers_dir.join("common");
        for d in [&server_path, &nested_common, &flat_common] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(nested_common.join("pyproject.toml"), "[project]").unwrap();
        std::fs::write(flat_common.join("pyproject.toml"), "[project]").unwrap();
        assert_eq!(
            resolve_common_source(&server_path, &servers_dir),
            Some(nested_common)
        );
    }

    #[test]
    fn common_source_falls_back_to_flat_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let servers_dir = tmp.path().join("mcp-servers");
        let server_path = servers_dir.join("terminal");
        let flat_common = servers_dir.join("common");
        std::fs::create_dir_all(&server_path).unwrap();
        std::fs::create_dir_all(&flat_common).unwrap();
        std::fs::write(flat_common.join("pyproject.toml"), "[project]").unwrap();
        assert_eq!(
            resolve_common_source(&server_path, &servers_dir),
            Some(flat_common)
        );
    }

    #[test]
    fn common_source_requires_pyproject() {
        let tmp = tempfile::tempdir().unwrap();
        let servers_dir = tmp.path().join("mcp-servers");
        let server_path = servers_dir.join("terminal");
        let flat_common = servers_dir.join("common");
        std::fs::create_dir_all(&server_path).unwrap();
        std::fs::create_dir_all(&flat_common).unwrap();
        // plain package dir without build metadata (pre-v0.1.8 layout)
        std::fs::write(flat_common.join("__init__.py"), "").unwrap();
        assert_eq!(resolve_common_source(&server_path, &servers_dir), None);
    }

    #[test]
    fn effective_install_dir_falls_back_to_id_when_directory_empty() {
        assert_eq!(effective_install_dir(&entry("cpersona", "")), "cpersona");
    }

    #[test]
    fn effective_install_dir_prefers_explicit_directory() {
        assert_eq!(
            effective_install_dir(&entry("memory.cpersona", "cpersona")),
            "cpersona"
        );
    }

    #[test]
    fn effective_install_dir_preserves_dotted_id_when_directory_empty() {
        // Legacy monorepo-style ids (e.g. `memory.cpersona`) flow through
        // unchanged — uninstall's old `.replace('.', "-")` fallback would
        // have collapsed this to `memory-cpersona`, diverging from the
        // installer's on-disk write path. Keeping ids as-is keeps the
        // three sites in lock-step.
        assert_eq!(
            effective_install_dir(&entry("memory.cpersona", "")),
            "memory.cpersona"
        );
    }

    #[test]
    fn effective_install_dir_collapses_monorepo_relative_directory() {
        // bug-399 / Task #105: a stale catalog `directory` carrying the
        // monorepo-relative path must not double with the source `subdir`. Only
        // the final component is used as the on-disk install dir, so
        // mcp-servers/<dir>/<subdir> stays single-nested instead of
        // mcp-servers/servers/<name>/servers/<name>.
        assert_eq!(
            effective_install_dir(&entry("tool.websearch", "servers/websearch")),
            "websearch"
        );
        assert_eq!(
            effective_install_dir(&entry("tool.cscheduler", "servers/cscheduler/")),
            "cscheduler"
        );
        // Well-formed flat directories and ids are unchanged.
        assert_eq!(
            effective_install_dir(&entry("tool.websearch", "websearch")),
            "websearch"
        );
    }

    /// bug-366: an in-flight install task registered in `AppState.install_task`
    /// must be abortable by the shutdown path so its child uv/pip processes are
    /// reaped (via kill_on_drop) instead of orphaned.
    #[tokio::test]
    async fn bug366_shutdown_aborts_inflight_install_task() {
        let state = crate::test_utils::create_test_app_state(None).await;

        // Mirror install_handler / batch_install_handler: a long-running task
        // stored in the tracking slot.
        let handle = tokio::spawn(async {
            loop {
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
        });
        *state.install_task.lock().await = Some(handle);

        // Mirror the shutdown handler's abort path.
        let taken = state.install_task.lock().await.take();
        let handle = taken.expect("install_task should hold the in-flight handle");
        handle.abort();

        let join_err = handle
            .await
            .expect_err("an aborted task must not complete normally");
        assert!(
            join_err.is_cancelled(),
            "install task should be cancelled by shutdown"
        );
        assert!(
            state.install_task.lock().await.is_none(),
            "slot is cleared after take()"
        );
    }

    #[test]
    fn reject_existing_only_blocks_fresh_install_of_present_server() {
        // The only combination that rejects is an existing server with no
        // update flag (fresh install). An update of an existing server proceeds
        // (in-place re-vendor); install/update of an absent server proceeds.
        assert!(
            reject_existing(true, false),
            "fresh install of present server is rejected"
        );
        assert!(
            !reject_existing(true, true),
            "update of present server proceeds"
        );
        assert!(
            !reject_existing(false, false),
            "fresh install of absent server proceeds"
        );
        assert!(
            !reject_existing(false, true),
            "update of absent server falls through to install"
        );
    }

    #[tokio::test]
    async fn existing_env_returns_stored_env_and_empty_when_absent() {
        let state = crate::test_utils::create_test_app_state(None).await;

        // Absent server → empty map.
        assert!(existing_env(&state.pool, "nope").await.is_empty());

        // Stored env (e.g. a user-entered secret) is returned verbatim.
        let rec = crate::db::mcp::McpServerRecord {
            name: "cpersona".into(),
            command: "python".into(),
            env: r#"{"SECRET_KEY":"shh","EMBEDDING_MODE":"http"}"#.into(),
            marketplace_id: Some("cpersona".into()),
            installed_version: Some("2.4.19".into()),
            ..Default::default()
        };
        crate::db::mcp::save_mcp_server(&state.pool, &rec)
            .await
            .unwrap();

        let env = existing_env(&state.pool, "cpersona").await;
        assert_eq!(env.get("SECRET_KEY").map(String::as_str), Some("shh"));
        assert_eq!(env.get("EMBEDDING_MODE").map(String::as_str), Some("http"));
    }

    #[tokio::test]
    async fn update_upsert_preserves_grants_while_uninstall_removes_them() {
        let state = crate::test_utils::create_test_app_state(None).await;
        let pool = &state.pool;

        // Install v1 and grant it to an agent.
        let mut rec = crate::db::mcp::McpServerRecord {
            name: "cpersona".into(),
            command: "python".into(),
            env: r#"{"SECRET_KEY":"shh"}"#.into(),
            marketplace_id: Some("cpersona".into()),
            installed_version: Some("2.4.19".into()),
            ..Default::default()
        };
        crate::db::mcp::save_mcp_server(pool, &rec).await.unwrap();
        crate::db::mcp::save_access_control_entry(
            pool,
            &crate::db::mcp::AccessControlEntry {
                id: None,
                entry_type: crate::db::mcp::EntryType::ServerGrant,
                agent_id: "agent.test".into(),
                server_id: "cpersona".into(),
                tool_name: None,
                permission: crate::db::mcp::PermissionLevel::Allow,
                granted_by: Some("admin".into()),
                granted_at: "2026-01-01T00:00:00Z".into(),
                expires_at: None,
                justification: None,
                metadata: None,
            },
        )
        .await
        .unwrap();

        // In-place update = re-run the UPSERT with a bumped version. This is the
        // path the update endpoint takes (save_mcp_server, never remove_server).
        rec.installed_version = Some("2.4.30".into());
        crate::db::mcp::save_mcp_server(pool, &rec).await.unwrap();

        let agents = crate::db::mcp::get_agents_for_server(pool, "cpersona")
            .await
            .unwrap();
        assert!(
            agents.contains(&"agent.test".to_string()),
            "in-place update must preserve grants"
        );
        let version = crate::db::mcp::load_active_mcp_servers(pool)
            .await
            .unwrap()
            .into_iter()
            .find(|s| s.name == "cpersona")
            .and_then(|s| s.installed_version);
        assert_eq!(
            version.as_deref(),
            Some("2.4.30"),
            "version is bumped in place"
        );

        // Contrast: the uninstall path DELETES grants (FK ON DELETE CASCADE) —
        // this is exactly why update must use the UPSERT path, not remove_server.
        crate::db::mcp::delete_mcp_server(pool, "cpersona")
            .await
            .unwrap();
        let after = crate::db::mcp::get_agents_for_server(pool, "cpersona")
            .await
            .unwrap();
        assert!(after.is_empty(), "uninstall removes grants");
    }
}
