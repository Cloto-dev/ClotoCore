//! ClotoCore kernel — an AI agent orchestration platform.
//!
//! Provides the Axum HTTP server, event-driven plugin system, SQLite persistence,
//! MCP server management, and the agentic loop that ties agents to reasoning engines.

pub mod apikey;
pub mod capabilities;
pub mod cli;
pub mod config;
pub mod consensus;
pub mod db;
pub mod defender;
pub mod events;
pub mod handlers;
pub mod installer;
pub mod managers;
pub mod middleware;
pub mod platform;
pub mod plugins;
pub mod test_utils;
pub mod viseme;

// Re-export audit log and permission request types for external use
pub use db::{
    create_permission_request, get_pending_permission_requests, is_permission_approved,
    query_audit_logs, update_permission_request, write_audit_log, AuditLogEntry, PermissionRequest,
};

// ── Shared timeout constants ────────────────────────────────────────────
// Durations that are shared across multiple kernel modules. Module-local
// timeouts with narrower semantics (e.g. AGENTIC_THINK_TIMEOUT_SECS,
// SSE_KEEPALIVE_INTERVAL_SECS) live next to their consumer.

/// Maximum wall time to wait for a spawned child process (uv install,
/// venv creation, migration, health probe). Long uploads and solid-state
/// installs legitimately take this long, but anything beyond is stuck.
pub(crate) const CHILD_PROCESS_TIMEOUT_SECS: u64 = 120;

/// Maximum wall time to download a tarball archive (registry snapshot or
/// per-server install bundle). Applies to the reqwest client-level timeout.
pub(crate) const TARBALL_DOWNLOAD_TIMEOUT_SECS: u64 = 120;

/// How long to wait for the HTTP server's readiness `Notify` before booting
/// deferred MCP servers. If we time out, we still boot — MCP callbacks just
/// log a warning until the server actually binds.
const HTTP_READY_WAIT_SECS: u64 = 30;

/// Initialize the global tracing subscriber.
///
/// In dev builds (`debug_assertions` on, `cfg(test)` off) this also writes a
/// daily-rotated `cloto-kernel.log` under `{exe_dir}/data/` alongside the stderr
/// stream, so post-mortem diagnosis is possible without re-running the session.
/// Release builds and test contexts emit to stderr only. The `WorkerGuard` for
/// the non-blocking file writer is intentionally leaked to give it a `'static`
/// lifetime — we accept that the final batch of lines may not flush on abrupt
/// termination in exchange for not having to thread the guard through every
/// embedder (main.rs, Tauri lib.rs).
///
/// Safe to call multiple times: uses `try_init`, so subsequent calls are no-ops.
/// Respects `RUST_LOG` via `EnvFilter`, defaulting to `info`.
#[cfg(all(debug_assertions, not(test)))]
pub fn init_tracing() {
    use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

    let data_dir = config::data_dir();
    // Best-effort: if we can't create the directory, fall back to stderr-only below.
    let file_layer = match std::fs::create_dir_all(&data_dir) {
        Ok(()) => {
            let appender = tracing_appender::rolling::daily(&data_dir, "cloto-kernel.log");
            let (writer, guard) = tracing_appender::non_blocking(appender);
            // Leak the guard so flushes continue for the life of the process.
            Box::leak(Box::new(guard));
            Some(fmt::layer().with_writer(writer).with_ansi(false))
        }
        Err(e) => {
            eprintln!(
                "[init_tracing] Failed to create data directory at {} ({}). File logging disabled.",
                data_dir.display(),
                e
            );
            None
        }
    };

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let base = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt::layer().with_writer(std::io::stderr));

    let _ = match file_layer {
        Some(fl) => base.with(fl).try_init(),
        None => base.try_init(),
    };
}

/// Release / test variant: stderr only, no file output.
#[cfg(not(all(debug_assertions, not(test))))]
pub fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let _ = fmt()
        .with_env_filter(env_filter)
        .with_writer(std::io::stderr)
        .try_init();
}

/// Rate limiter stale-entry cleanup interval in seconds.
const RATE_LIMITER_CLEANUP_SECS: u64 = 600;

/// Revoked API keys TTL cleanup interval in seconds (6 hours).
const REVOKED_KEYS_CLEANUP_SECS: u64 = 21_600;

use cloto_shared::ClotoEvent;
use sqlx::SqlitePool;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU8};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Notify, RwLock};

/// Context for a currently-executing CRON job (tracks generation for recursion depth).
#[derive(Debug, Clone)]
pub struct CronExecContext {
    pub job_id: String,
    pub generation: i32,
}

/// Per-agent active CRON execution contexts (agent_id → context).
pub type ActiveCronContexts = Arc<dashmap::DashMap<String, CronExecContext>>;

#[derive(Debug, Clone)]
pub struct EnvelopedEvent {
    pub event: Arc<ClotoEvent>,
    pub issuer: Option<cloto_shared::ClotoId>, // None = System/Kernel
    pub correlation_id: Option<cloto_shared::ClotoId>, // trace_id of the parent event
    pub depth: u8,
}

impl EnvelopedEvent {
    /// Create a system-originated event (no issuer, no correlation, depth 0)
    #[must_use]
    pub fn system(data: cloto_shared::ClotoEventData) -> Self {
        Self {
            event: Arc::new(ClotoEvent::new(data)),
            issuer: None,
            correlation_id: None,
            depth: 0,
        }
    }
}

pub struct DynamicRouter {
    pub router: RwLock<axum::Router<Arc<dyn std::any::Any + Send + Sync>>>,
}

pub struct AppState {
    pub tx: broadcast::Sender<events::SequencedEvent>,
    pub registry: Arc<managers::PluginRegistry>,
    pub event_tx: mpsc::Sender<EnvelopedEvent>,
    pub pool: SqlitePool,
    pub agent_manager: managers::AgentManager,
    pub plugin_manager: Arc<managers::PluginManager>,
    pub mcp_manager: Arc<managers::McpClientManager>,
    pub dynamic_router: Arc<DynamicRouter>,
    pub config: config::AppConfig,
    pub data_dir: std::path::PathBuf,
    pub event_history: Arc<RwLock<VecDeque<events::SequencedEvent>>>,
    pub metrics: Arc<managers::SystemMetrics>,
    pub rate_limiter: Arc<middleware::RateLimiter>,
    pub shutdown: Arc<Notify>,
    /// In-memory cache of revoked API key hashes (SHA-256 fingerprints).
    /// Loaded from DB at startup; updated on POST /api/system/invalidate-key.
    pub revoked_keys: Arc<tokio::sync::RwLock<std::collections::HashSet<String>>>,
    /// Live admin API key. Seeded from `config.admin_api_key` at boot and
    /// swapped in place by POST /api/system/regenerate-key so a rotation
    /// takes effect without a restart (std RwLock: sync readers in check_auth).
    pub admin_api_key: std::sync::RwLock<Option<String>>,
    /// Pending command approval requests (kernel ↔ API handler bridge).
    pub pending_command_approvals: handlers::command_approval::PendingApprovals,
    /// Session-scoped trusted command names (cleared on restart).
    pub session_trusted_commands: handlers::command_approval::SessionTrustedCommands,
    /// Per-agent active CRON execution contexts (for recursion depth tracking).
    pub active_cron_contexts: ActiveCronContexts,
    /// Maximum allowed CRON recursion depth (0-6, default 2).
    pub max_cron_generation: Arc<AtomicU8>,
    /// Whether a bootstrap setup is currently running.
    pub setup_in_progress: Arc<AtomicBool>,
    /// Handle to the in-flight install task (bootstrap or marketplace), tracked so
    /// shutdown can abort it and reap orphaned child uv/pip processes (bug-366).
    pub install_task: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    /// Whether initial setup (venv, servers) has been completed at least once.
    /// Starts `false` on first run; set `true` when batch install finishes.
    /// Used by health monitor to suppress auto-restart before setup.
    pub setup_done: Arc<AtomicBool>,
    /// Broadcast channel for setup progress events (SSE).
    pub setup_progress_tx: broadcast::Sender<handlers::setup::SetupProgressEvent>,
    /// In-memory cache for marketplace catalog (registry.json).
    pub marketplace_cache: Arc<tokio::sync::RwLock<handlers::marketplace::CatalogCache>>,
    /// In-memory cache of the hub's Ed25519 seal-signing keys (JWKS),
    /// used to verify catalog seal signatures at install time (bug-394
    /// proper fix).
    pub seal_jwks_cache: Arc<tokio::sync::RwLock<handlers::marketplace::JwksCache>>,
    /// Stricter rate limiter for heavy operations (install, setup).
    /// 5 req/min per IP to prevent GitHub API abuse and disk exhaustion.
    pub install_limiter: Arc<middleware::RateLimiter>,
    /// Cached result from the last health scan (populated at startup and on-demand).
    pub last_health_report: Arc<tokio::sync::RwLock<Option<db::health::HealthReport>>>,
    /// 10-second TTL cache of LM Studio probe results (runtime metadata per provider).
    /// Only populated when the dashboard requests the model dropdown for a local provider.
    pub provider_probe_cache: managers::provider_probe::ProbeCache,
    /// Ephemeral per-agent store of the most recent response's token usage.
    /// Populated by the agentic loop on each completion; read by the dashboard
    /// "context usage" badge.
    pub last_usage: managers::usage_tracker::UsageStore,
    /// In-flight conversation state (T1, v0.6.3+) keyed by
    /// `(agent_id, bridge_session_id)`. Process-lifetime only — see
    /// `managers::session_manager` for the tier model and rationale.
    pub session_manager: Arc<managers::session_manager::SessionManager>,
}

pub enum AppError {
    Cloto(cloto_shared::ClotoError),
    Internal(anyhow::Error),
    NotFound(String),
    Validation(String),
    /// The request was well-formed but the system is not in a state to carry it
    /// out — and the caller needs to be told which, in words. Distinct from
    /// `Validation` (the request was wrong) and from `Internal` (whose message
    /// is deliberately withheld from the client), because the states this
    /// covers are ones a user resolves themselves: a declined elevation
    /// prompt, an uninstall with nothing to remove.
    Conflict(String),
    Mgp(Box<managers::mcp_mgp::MgpError>),
}

impl axum::response::IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let (status, err_type, message) = match self {
            AppError::Cloto(e) => {
                let status = match &e {
                    cloto_shared::ClotoError::PermissionDenied(_) => {
                        axum::http::StatusCode::FORBIDDEN
                    }
                    cloto_shared::ClotoError::PluginNotFound(_)
                    | cloto_shared::ClotoError::AgentNotFound(_) => {
                        axum::http::StatusCode::NOT_FOUND
                    }
                    _ => axum::http::StatusCode::BAD_REQUEST,
                };
                (status, format!("{:?}", e), e.to_string())
            }
            AppError::Internal(e) => {
                // Log full error server-side only; return generic message to client
                tracing::error!("Internal error: {}", e);
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "InternalError".to_string(),
                    "An internal error occurred".to_string(),
                )
            }
            AppError::NotFound(m) => (axum::http::StatusCode::NOT_FOUND, "NotFound".to_string(), m),
            AppError::Validation(m) => (
                axum::http::StatusCode::BAD_REQUEST,
                "ValidationError".to_string(),
                m,
            ),
            AppError::Conflict(m) => (axum::http::StatusCode::CONFLICT, "Conflict".to_string(), m),
            AppError::Mgp(ref e) => {
                let status = match e.code {
                    1000 | 1001 | 1010 | 1011 => axum::http::StatusCode::FORBIDDEN,
                    1002 | 1003 => axum::http::StatusCode::UNAUTHORIZED,
                    2000..=2002 => axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    3000 | 3002 => axum::http::StatusCode::TOO_MANY_REQUESTS,
                    3003 | 5001 => axum::http::StatusCode::GATEWAY_TIMEOUT,
                    4000 => axum::http::StatusCode::BAD_REQUEST,
                    4001..=4003 | 4100..=4102 => axum::http::StatusCode::NOT_FOUND,
                    _ => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                };
                let body = axum::Json(serde_json::json!({
                    "error": e.to_json_rpc_error()
                }));
                return (status, body).into_response();
            }
        };

        let body = axum::Json(serde_json::json!({
            "error": {
                "type": err_type,
                "message": message
            }
        }));

        (status, body).into_response()
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::Internal(err)
    }
}

impl From<managers::mcp_mgp::MgpError> for AppError {
    fn from(err: managers::mcp_mgp::MgpError) -> Self {
        AppError::Mgp(Box::new(err))
    }
}

impl From<cloto_shared::ClotoError> for AppError {
    fn from(err: cloto_shared::ClotoError) -> Self {
        AppError::Cloto(err)
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        AppError::Internal(anyhow::anyhow!(err))
    }
}

pub type AppResult<T> = Result<T, AppError>;

/// Handle returned by [`start_kernel`] — keeps the kernel alive.
///
/// Dropping this handle does **not** trigger shutdown; use [`KernelHandle::shutdown`]
/// or the `/api/system/shutdown` endpoint instead.
pub struct KernelHandle {
    /// Notify to trigger graceful shutdown of the HTTP server and background tasks.
    pub shutdown: Arc<Notify>,
    /// The MCP client manager, exposed so an embedder (e.g. Tauri) can drain and
    /// reap MCP subprocesses on app exit instead of orphaning them
    /// (orphan-leak fix, Step 4). Use `mcp_manager.drain_all(...)` before exit.
    pub mcp_manager: Arc<managers::McpClientManager>,
    /// Join handle for the HTTP server task.
    server_task: tokio::task::JoinHandle<()>,
}

/// Connect to the SQLite pool and run migrations/seeds. Factored out of
/// [`start_kernel`] so [`open_kernel_db`] can retry it after quarantining a
/// corrupt DB, and so tests can exercise it directly.
async fn connect_and_init_db(
    database_url: &str,
    memory_plugin_id: Option<&str>,
) -> anyhow::Result<sqlx::SqlitePool> {
    use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};
    use std::str::FromStr;

    // WAL journal + busy_timeout are critical for concurrent write resilience:
    // SQLite's default DELETE journal serializes readers against writers, and a
    // default busy_timeout of 0 fails SQLITE_BUSY immediately. Before this change
    // the audit_logs writer (which uses a two-statement tx to chain-hash the
    // previous row) went silent for 14 h under normal load because every retry
    // lost the race while chat_messages kept squeezing through. WAL lets readers
    // run in parallel with a writer, and busy_timeout=10s gives the retry ladder
    // in spawn_audit_log a realistic chance to succeed.
    let opts = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(std::time::Duration::from_secs(10))
        .pragma("foreign_keys", "ON")
        .pragma("synchronous", "NORMAL");
    let pool = sqlx::SqlitePool::connect_with(opts).await?;
    crate::db::init_db(&pool, database_url, memory_plugin_id).await?;
    Ok(pool)
}

/// If `err` is the recoverable corrupt-DB class (SQLite `SQLITE_NOTADB` /
/// "file is not a database" / "database disk image is malformed") **and**
/// `database_url` points at an on-disk SQLite file that exists, return that
/// file's path. Otherwise return `None` — genuinely unrecoverable errors
/// (permission denied, disk full, an in-memory URL, or a missing file) must
/// propagate so the caller still hard-fails.
fn recoverable_corrupt_db_path(
    database_url: &str,
    err: &anyhow::Error,
) -> Option<std::path::PathBuf> {
    // Only file-backed sqlite: URLs can be quarantined. Reject in-memory forms.
    let path_str = database_url.strip_prefix("sqlite:")?;
    if path_str.is_empty() || path_str.starts_with(":memory:") || path_str.contains("mode=memory") {
        return None;
    }
    // Drop any `?query` parameters to get the bare filesystem path.
    let path_str = path_str.split('?').next().unwrap_or(path_str);

    let is_corrupt = err.chain().any(|cause| {
        if let Some(sqlx::Error::Database(db)) = cause.downcast_ref::<sqlx::Error>() {
            if db.code().as_deref() == Some("26") {
                return true; // SQLITE_NOTADB
            }
        }
        let m = cause.to_string().to_ascii_lowercase();
        m.contains("file is not a database")
            || m.contains("database disk image is malformed")
            || m.contains("file is encrypted or is not a database")
    });
    if !is_corrupt {
        return None;
    }

    let path = std::path::Path::new(path_str);
    path.exists().then(|| path.to_path_buf())
}

/// Rename a corrupt SQLite DB file — and its `-wal` / `-shm` sidecars — aside
/// with a timestamped `.corrupt-<ts>.bak` suffix so a fresh DB can be created
/// in its place. **Never deletes** (Destructive DB rule): the unreadable data
/// is preserved for post-mortem / manual recovery. Returns the backup path.
fn quarantine_corrupt_db(db_path: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let file_name = db_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("cloto_memories.db");
    let backup = db_path.with_file_name(format!("{file_name}.corrupt-{ts}.bak"));
    std::fs::rename(db_path, &backup)?;
    // Stale WAL/SHM sidecars would corrupt the freshly created DB — move them
    // aside too (best-effort; their absence is fine).
    for ext in ["-wal", "-shm"] {
        let side = std::path::PathBuf::from(format!("{}{ext}", db_path.display()));
        if side.exists() {
            let side_backup = std::path::PathBuf::from(format!("{}{ext}", backup.display()));
            let _ = std::fs::rename(&side, &side_backup);
        }
    }
    Ok(backup)
}

/// Open the kernel SQLite pool and run migrations, self-healing a corrupt /
/// non-SQLite database file (bug-486).
///
/// A real user can reach `SQLITE_NOTADB` (SQLite error code 26, "file is not a
/// database") from disk corruption, an interrupted/torn write, or AV quarantine
/// of the DB. Previously any such error propagated out of [`start_kernel`] to a
/// fatal "Cloto Kernel failed to start" dialog + `exit(1)`, leaving the user
/// permanently unable to launch with no in-app remedy.
///
/// On the first-open failing with the recoverable corrupt-DB class, the
/// unreadable file (and its `-wal`/`-shm` sidecars) is renamed aside with a
/// timestamped `.corrupt-*.bak` suffix (never deleted) and a fresh DB is created
/// and migrated in its place, so the app launches instead of dead-ending. Any
/// other error — or a second failure after recovery — propagates unchanged, so
/// genuinely unrecoverable conditions still surface the fatal dialog.
pub async fn open_kernel_db(
    database_url: &str,
    memory_plugin_id: Option<&str>,
) -> anyhow::Result<sqlx::SqlitePool> {
    use anyhow::Context;

    match connect_and_init_db(database_url, memory_plugin_id).await {
        Ok(pool) => Ok(pool),
        Err(e) => {
            let Some(db_path) = recoverable_corrupt_db_path(database_url, &e) else {
                return Err(e);
            };
            let backup = quarantine_corrupt_db(&db_path).with_context(|| {
                format!(
                    "kernel DB at {} is corrupt but could not be moved aside for recovery",
                    db_path.display()
                )
            })?;
            tracing::error!(
                backup = %backup.display(),
                "🩹 Kernel database was corrupt or not a valid SQLite file; moved it \
                 aside and recreating a fresh database. Prior data (if any) is preserved \
                 in the backup file — the app will launch with an empty database."
            );
            connect_and_init_db(database_url, memory_plugin_id)
                .await
                .context("re-initializing a fresh kernel DB after quarantining the corrupt one")
        }
    }
}

/// Initialize the kernel (DB, plugins, MCP, LLM proxy, event loop) and spawn the
/// HTTP server in the background.  Returns a [`KernelHandle`] on success.
///
/// Use this from Tauri (or other embedders) when you need to detect startup failures
/// **before** showing the UI.  For standalone CLI usage, prefer [`run_kernel`] which
/// blocks until shutdown.
#[allow(clippy::too_many_lines)]
pub async fn start_kernel() -> anyhow::Result<KernelHandle> {
    use crate::config::AppConfig;
    use crate::db;
    use crate::events::EventProcessor;
    use crate::handlers::{self, system::SystemHandler};
    use crate::managers::{AgentManager, PluginManager};
    use axum::{
        routing::{any, delete, get, post},
        Router,
    };
    use tower_http::cors::CorsLayer;
    use tracing::info;

    let kernel_start = std::time::Instant::now();

    info!("+---------------------------------------+");
    info!("|             ClotoCore Kernel           |");
    info!(
        "|             Version {:<10}      |",
        env!("CARGO_PKG_VERSION")
    );
    info!("+---------------------------------------+");

    // Experimental-build warning (docs/RELEASE_PIPELINE_DESIGN.md §6) — a semver
    // pre-release suffix is locally derivable, no network needed.
    if env!("CARGO_PKG_VERSION").contains('-') {
        tracing::warn!(
            "⚠️  Experimental build v{} — opt-in, no guarantees; fixes ship in the next pre-release",
            env!("CARGO_PKG_VERSION")
        );
    }

    let config = AppConfig::load()?;
    // H-06: Mask DB path in logs (show filename only, not full path)
    let db_display = config
        .database_url
        .rsplit_once('/')
        .or_else(|| config.database_url.rsplit_once('\\'))
        .map_or("***", |(_, name)| name);
    info!(
        "📍 Loaded Config: DB={}, DEFAULT_AGENT={}",
        db_display, config.default_agent_id
    );
    // Full DB path at debug level for troubleshooting persistence issues
    tracing::debug!("📍 DB full path: {}", config.database_url);
    tracing::debug!("📍 exe_dir resolved to: {}", config::exe_dir().display());

    // Principle #5: a missing admin API key is a warning on loopback and a
    // refusal to start anywhere else. On a non-loopback bind the key is the
    // only boundary between the admin API and every host that can reach this
    // address; starting without one would answer 403 to everything while
    // looking, from the outside, like a live unauthenticated kernel.
    let bind_is_loopback = config.bind_is_loopback();
    if config.admin_api_key.is_none() {
        if !bind_is_loopback && !unauthenticated_http_allowed() {
            anyhow::bail!(
                "CLOTO_API_KEY is not set and BIND_ADDRESS={} is reachable from other hosts. \
                 Refusing to start. Set CLOTO_API_KEY (the installer writes one into .env), \
                 or set CLOTO_ALLOW_UNAUTHENTICATED_HTTP=1 to start a listener that rejects \
                 every admin request.",
                config.bind_address
            );
        }
        if !cfg!(debug_assertions) {
            tracing::warn!(
                "⚠️  CLOTO_API_KEY is not set. All admin endpoints will reject requests."
            );
            tracing::warn!(
                "    Set CLOTO_API_KEY in .env or environment to enable admin operations."
            );
        }
    }
    if !bind_is_loopback {
        tracing::warn!(
            "⚠️  BIND_ADDRESS={} is reachable from other hosts. The admin API key is the only \
             boundary on this listener; keep it secret and rotate it if it leaks.",
            config.bind_address
        );
    }

    // 0. Ensure parent directory of DB file exists (for deployed layout)
    if let Some(path_str) = config.database_url.strip_prefix("sqlite:") {
        let db_path = std::path::Path::new(path_str);
        if let Some(parent) = db_path.parent() {
            if !parent.as_os_str().is_empty() && parent != std::path::Path::new(".") {
                std::fs::create_dir_all(parent)?;
                // Restrict data directory permissions (contains SQLite DB)
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ =
                        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
                }
                info!("📁 Data directory: {}", parent.display());
            }
        }
    }

    // 0b. Ensure storage directories exist (user-writable layout per bug-377).
    let data_dir = config::data_dir();
    if let Err(e) = std::fs::create_dir_all(data_dir.join("attachments")) {
        tracing::warn!("Failed to create data/attachments directory: {}", e);
    }
    if let Err(e) = std::fs::create_dir_all(data_dir.join("avatars")) {
        tracing::warn!("Failed to create data/avatars directory: {}", e);
    }
    if let Err(e) = std::fs::create_dir_all(data_dir.join("vrm")) {
        tracing::warn!("Failed to create data/vrm directory: {}", e);
    }
    if let Err(e) = std::fs::create_dir_all(data_dir.join("speech")) {
        tracing::warn!("Failed to create data/speech directory: {}", e);
    }
    tracing::info!("📁 Data directory: {}", data_dir.display());

    // 0b'. Defender install receipt (DEFENDER_DESIGN.md §3): refresh the
    // ledger of kernel-managed paths. Best-effort — never blocks boot.
    // The pre-refresh app_version tells us whether this is the first boot of
    // a new version (§6 clean-update phase).
    let receipt_prev_version = defender::footprint::load(&data_dir).map(|r| r.app_version);
    defender::footprint::record(&data_dir, defender::footprint::boot_entries(&data_dir));
    if let Some(prev) = receipt_prev_version {
        if prev != env!("CARGO_PKG_VERSION") {
            defender::repair::first_boot_maintenance(&data_dir, &prev);
        }
    }

    // 0c. Ensure Python MCP venv exists (auto-setup on first run)
    // Skip in production if bootstrap setup has not been completed yet.
    let setup_json = data_dir.join("setup-complete.json");
    let is_dev = {
        let exe = std::env::current_exe().unwrap_or_default();
        managers::McpClientManager::detect_project_root(&exe)
            .is_some_and(|r| r.join("Cargo.toml").exists())
    };
    if setup_json.exists() || is_dev {
        // Run venv dependency sync in background — not on the critical startup path.
        // Servers can start immediately since the venv python and existing packages
        // are already available; pip install only adds/updates packages.
        let data_dir_bg = data_dir.clone();
        tokio::spawn(async move {
            managers::mcp_venv::ensure_mcp_venv(&data_dir_bg).await;
        });
    } else {
        tracing::info!("Setup not complete — skipping MCP venv sync");
    }

    // Probe the marketplace install engine so a missing or stale binary
    // is in the boot log and on the health endpoint before anyone
    // opens the marketplace. Off the critical path: it is re-probed by
    // every install, which is where it is enforced.
    tokio::spawn(async {
        let status = managers::installer::probe().await;
        match status.error {
            None => tracing::info!(
                "Marketplace install engine ready: {} ({})",
                status.path.display(),
                status.version.as_deref().unwrap_or("?")
            ),
            Some(error) => tracing::warn!("{error}"),
        }
    });

    // 0d. Set database timeout from config
    db::set_db_timeout(config.db_timeout_secs);

    // 1. Initialize the database.
    //
    // Opens the pool and runs migrations, self-healing a corrupt / non-SQLite
    // DB file (bug-486) instead of dead-ending the launch. See `open_kernel_db`.
    let pool = open_kernel_db(&config.database_url, config.memory_plugin_id.as_deref()).await?;

    // 1b. Sync API keys from environment variables into llm_providers table
    db::sync_env_api_keys(&pool, &config.llm_provider_env_mappings).await;

    // 2. Plugin Manager Setup
    let shutdown = Arc::new(Notify::new());
    // P1: Merge network whitelist + API host whitelist for SafeHttpClient
    let mut all_allowed_hosts = config.allowed_hosts.clone();
    all_allowed_hosts.extend(config.default_allowed_api_hosts.clone());
    let mut plugin_manager_obj = PluginManager::new(
        pool.clone(),
        all_allowed_hosts,
        config.plugin_event_timeout_secs,
        config.max_event_depth,
        config.event_concurrency_limit,
    )?;
    plugin_manager_obj.shutdown = shutdown.clone();

    // 3. Channel Setup
    let (event_tx, event_rx) = tokio::sync::mpsc::channel::<EnvelopedEvent>(100);
    plugin_manager_obj.set_event_tx(event_tx.clone());
    let plugin_manager = Arc::new(plugin_manager_obj);

    // 3b. MCP Client Manager (created early so PluginRegistry can reference it)
    // Resolve YOLO mode: DB-persisted value takes precedence over env var
    let yolo_mode = {
        let db_yolo: Option<(String,)> = sqlx::query_as(
            "SELECT config_value FROM plugin_configs WHERE plugin_id = 'kernel' AND config_key = 'yolo_mode'"
        )
            .fetch_optional(&pool)
            .await
            .unwrap_or(None);
        match db_yolo {
            Some((val,)) => val == "true",
            None => config.yolo_mode, // fall back to env var
        }
    };
    let mut mcp_manager = managers::McpClientManager::new(
        pool.clone(),
        yolo_mode,
        config.mcp_request_timeout_secs,
        config.mcp_stream_idle_timeout_secs,
    );
    mcp_manager.configure_isolation(&config);
    let mcp_manager = Arc::new(mcp_manager);

    // 4. Initialize External Plugins
    let mut registry = plugin_manager.initialize_all()?;
    registry.set_mcp_manager(mcp_manager.clone());
    let registry_arc = Arc::new(registry);

    // 5. Managers & Internal Handlers
    let agent_manager = AgentManager::new(pool.clone(), config.heartbeat_threshold_ms);
    let (tx, _rx) = tokio::sync::broadcast::channel::<events::SequencedEvent>(100);

    let dynamic_router = Arc::new(DynamicRouter {
        router: tokio::sync::RwLock::new(Router::new()),
    });

    let metrics = Arc::new(managers::SystemMetrics::new());
    let event_history = Arc::new(tokio::sync::RwLock::new(VecDeque::new()));

    // Register the System Handler.
    let pending_command_approvals: handlers::command_approval::PendingApprovals =
        Arc::new(dashmap::DashMap::new());
    let session_trusted_commands: handlers::command_approval::SessionTrustedCommands =
        Arc::new(dashmap::DashMap::new());
    let active_cron_contexts: ActiveCronContexts = Arc::new(dashmap::DashMap::new());
    let max_cron_generation = Arc::new(AtomicU8::new(
        std::env::var("CLOTO_MAX_CRON_GENERATION")
            .ok()
            .and_then(|v| v.parse::<u8>().ok())
            .map_or(2, |v| v.min(6)),
    ));

    // Shared between SystemHandler (reader — consults LM Studio's actual loaded n_ctx
    // to clamp DB-configured context_length during pre-flight) and AppState
    // (populated by /api/llm/providers/:id/models requests).
    let probe_cache = managers::provider_probe::ProbeCache::new();

    // Shared between SystemHandler (writer — records usage after each LLM call) and
    // AppState (reader — exposes GET /api/agents/:id/last-usage to the dashboard).
    let last_usage_store = managers::usage_tracker::UsageStore::new();

    // Shared T1 conversation state (v0.6.3+) — SystemHandler reads / mutates
    // it during the agentic loop; a background cleanup task below evicts
    // Cold sessions and clears stale tool_history on the Warm tier boundary.
    let session_manager = Arc::new(managers::session_manager::SessionManager::new());

    // Consensus knobs (kernel-level; replaces the retired core.moderator plugin).
    // The orchestration runs in-kernel inside SystemHandler::run_consensus —
    // see docs/CONSENSUS_REVIVAL_DESIGN.md.
    let consensus_config = consensus::ConsensusConfig {
        synthesizer_engine: std::env::var("CONSENSUS_SYNTHESIZER").unwrap_or_default(),
        synthetic_agent_id: std::env::var("CONSENSUS_AGENT_ID")
            .unwrap_or_else(|_| consensus::DEFAULT_CONSENSUS_AGENT_ID.to_string()),
        min_proposals: std::env::var("CONSENSUS_MIN_PROPOSALS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2)
            .max(2),
        session_timeout_secs: std::env::var("CONSENSUS_SESSION_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60)
            .max(10),
        // Default on: re-sample a working engine to reach quorum when too few
        // distinct engines are available, rather than hard-failing.
        engine_reuse: match std::env::var("CONSENSUS_ENGINE_REUSE") {
            Ok(v) => !matches!(
                v.trim().to_lowercase().as_str(),
                "0" | "false" | "off" | "no"
            ),
            Err(_) => true,
        },
    };

    let system_handler = {
        let mut h = SystemHandler::new(
            registry_arc.clone(),
            agent_manager.clone(),
            config.default_agent_id.clone(),
            event_tx.clone(),
            config.memory_context_limit,
            metrics.clone(),
            config.consensus_engines.clone(),
            config.consensus_prefix.clone(),
            config.max_agentic_iterations,
            config.tool_execution_timeout_secs,
            pending_command_approvals.clone(),
            session_trusted_commands.clone(),
            pool.clone(),
            active_cron_contexts.clone(),
            config.memory_timeout_secs,
            config.mcp_streaming_enabled,
        );
        h.set_probe_cache(probe_cache.clone());
        h.set_usage_store(last_usage_store.clone());
        h.set_session_manager(session_manager.clone());
        h.set_consensus_config(consensus_config);
        Arc::new(h)
    };

    // Background T1 cleanup — 60s tick is fine; tier transitions happen on
    // minute timescales and the cost per tick is `O(active sessions)`.
    {
        let sm = session_manager.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_mins(1));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let evicted = sm.run_cleanup();
                if evicted > 0 {
                    tracing::debug!(evicted, "T1 SessionManager: cold-evicted sessions");
                }
            }
        });
    }

    // SystemHandler is NOT registered as a plugin — it runs outside the dispatch
    // pipeline to avoid blocking the event loop during agentic loops.
    // It is passed directly to EventProcessor instead.

    // One-time migration: mcp.toml → DB (if mcp.toml exists)
    if let Err(e) = mcp_manager.migrate_config_file_to_db(&data_dir).await {
        tracing::warn!(error = %e, "mcp.toml migration failed — continuing with DB only");
    }

    // Load MCP servers from DB.
    // Priority boot: connect default agent's granted servers first, defer the rest.
    let deferred_mcp_configs = {
        match mcp_manager
            .load_and_connect_priority(&config.default_agent_id, &agent_manager)
            .await
        {
            Ok(deferred) => deferred,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "Failed to load MCP servers from database"
                );
                Vec::new()
            }
        }
    };

    // 5. Rate Limiter & App State
    let rate_limiter = Arc::new(middleware::RateLimiter::new(
        config.rate_limit_per_sec,
        config.rate_limit_burst,
    ));
    // Stricter limiter for heavy operations (marketplace install, batch setup)
    let install_limiter = Arc::new(middleware::RateLimiter::per_minute(5, 5));

    // Load revoked key hashes into memory
    let revoked_keys = {
        let mut set = std::collections::HashSet::new();
        match db::load_revoked_key_hashes(&pool).await {
            Ok(hashes) => {
                let count = hashes.len();
                set.extend(hashes);
                if count > 0 {
                    info!(count = count, "🔑 Loaded revoked API key hashes");
                }
            }
            Err(e) => tracing::warn!(error = %e, "Failed to load revoked key hashes"),
        }
        Arc::new(tokio::sync::RwLock::new(set))
    };

    let app_state = Arc::new(AppState {
        tx: tx.clone(),
        registry: registry_arc.clone(),
        event_tx: event_tx.clone(),
        pool: pool.clone(),
        agent_manager: agent_manager.clone(),
        plugin_manager: plugin_manager.clone(),
        mcp_manager: mcp_manager.clone(),
        dynamic_router: dynamic_router.clone(),
        admin_api_key: std::sync::RwLock::new(config.admin_api_key.clone()),
        config: config.clone(),
        data_dir: data_dir.clone(),
        event_history: event_history.clone(),
        metrics: metrics.clone(),
        rate_limiter: rate_limiter.clone(),
        shutdown,
        revoked_keys,
        pending_command_approvals,
        session_trusted_commands,
        active_cron_contexts,
        max_cron_generation,
        setup_in_progress: Arc::new(AtomicBool::new(false)),
        install_task: Arc::new(tokio::sync::Mutex::new(None)),
        setup_done: Arc::new(AtomicBool::new(setup_json.exists() || is_dev)),
        setup_progress_tx: {
            let (tx, _) = broadcast::channel(64);
            tx
        },
        marketplace_cache: Arc::new(tokio::sync::RwLock::new(
            handlers::marketplace::CatalogCache::default(),
        )),
        seal_jwks_cache: Arc::new(tokio::sync::RwLock::new(
            handlers::marketplace::JwksCache::default(),
        )),
        install_limiter: install_limiter.clone(),
        last_health_report: Arc::new(tokio::sync::RwLock::new(None)),
        provider_probe_cache: probe_cache,
        last_usage: last_usage_store,
        session_manager,
    });

    // Wire up kernel event bus to MCP manager (for PermissionRequested emission)
    mcp_manager.set_kernel_event_tx(event_tx.clone()).await;

    // 6a. Event Loop
    let processor = Arc::new(EventProcessor::new(
        registry_arc.clone(),
        plugin_manager.clone(),
        agent_manager.clone(),
        tx.clone(),
        event_history,
        metrics,
        config.event_history_size,
        config.event_retention_hours,
        system_handler,
        config.max_event_history,
        config.hal_rate_limit_per_sec,
        config.hal_rate_limit_burst,
    ));

    // Start event history cleanup task
    processor
        .clone()
        .spawn_cleanup_task(app_state.shutdown.clone());

    // 6a. Active Heartbeat task (ping all enabled agents every 30s)
    let heartbeat_interval = std::env::var("HEARTBEAT_INTERVAL_SECS")
        .unwrap_or_else(|_| "30".to_string())
        .parse::<u64>()
        .unwrap_or(30);
    EventProcessor::spawn_heartbeat_task(
        agent_manager.clone(),
        heartbeat_interval,
        app_state.shutdown.clone(),
    );

    // 6b. MCP deferred boot — connect non-priority servers in background
    //     Wait for HTTP server to be ready before connecting (MGP callbacks need it).
    let http_ready = Arc::new(Notify::new());
    if !deferred_mcp_configs.is_empty() {
        let deferred_mcp = mcp_manager.clone();
        let deferred_shutdown = app_state.shutdown.clone();
        let deferred_http_ready = http_ready.clone();
        tokio::spawn(async move {
            // Wait for HTTP server to bind before connecting deferred MCP servers,
            // because they may send MGP callbacks that hit kernel HTTP endpoints.
            if tokio::time::timeout(
                std::time::Duration::from_secs(HTTP_READY_WAIT_SECS),
                deferred_http_ready.notified(),
            )
            .await
            .is_err()
            {
                tracing::warn!(
                    "HTTP server readiness timed out ({HTTP_READY_WAIT_SECS}s), proceeding with deferred MCP boot"
                );
            }
            info!(
                count = deferred_mcp_configs.len(),
                "🔌 Background: connecting deferred MCP servers"
            );
            deferred_mcp
                .connect_server_configs(&deferred_mcp_configs)
                .await;
            let _ = &deferred_shutdown; // hold reference to prevent premature shutdown
            info!("✅ Background MCP server boot complete");
        });
    }

    // 6b2. MCP health monitor — auto-restart dead servers (bug-142)
    Arc::clone(&mcp_manager).spawn_health_monitor(
        app_state.shutdown.clone(),
        app_state.config.mcp_health_interval_secs,
        app_state.setup_in_progress.clone(),
        app_state.setup_done.clone(),
    );

    // 6b2. MCP notification listener — forward Server→Kernel notifications to event bus
    if let Some(mut notif_rx) = mcp_manager.take_notification_receiver().await {
        let notif_event_tx = event_tx.clone();
        let notif_mcp_manager = mcp_manager.clone();
        let shutdown_clone = app_state.shutdown.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    () = shutdown_clone.notified() => {
                        tracing::info!("MCP notification listener shutting down");
                        break;
                    }
                    notif_opt = notif_rx.recv() => {
                        let Some(notif) = notif_opt else { break };

                        // Intercept callback requests (MGP §13)
                        if notif.method == "notifications/mgp.callback.request" {
                            if let Some(ref params) = notif.params {
                                match managers::mcp::mcp_events_handle_callback(
                                    &notif_mcp_manager, &notif.server_id, params,
                                ) {
                                    managers::mcp::CallbackHandleResult::NewCallback(event_data) => {
                                        let envelope = EnvelopedEvent::system(*event_data);
                                        if let Err(e) = notif_event_tx.send(envelope).await {
                                            tracing::warn!("Failed to forward callback event: {}", e);
                                        }
                                    }
                                    managers::mcp::CallbackHandleResult::DuplicateWithResponse {
                                        server_id,
                                        callback_id,
                                        response,
                                    } => {
                                        let mgr = notif_mcp_manager.clone();
                                        tokio::spawn(async move {
                                            let state = mgr.state.read().await;
                                            if let Some(handle) = state.servers.get(&server_id) {
                                                if let Some(client) = &handle.client {
                                                    let params = serde_json::json!({
                                                        "callback_id": callback_id,
                                                        "response": response,
                                                    });
                                                    let _ = client.call("mgp/callback/respond", Some(params)).await;
                                                }
                                            }
                                        });
                                    }
                                    managers::mcp::CallbackHandleResult::DuplicateNoResponse => {}
                                }
                            }
                            continue;
                        }

                        // Intercept stream chunks for gap detection (MGP §12)
                        if notif.method == "notifications/mgp.stream.chunk" {
                            if let Some(ref params) = notif.params {
                                let request_id = params
                                    .get("request_id")
                                    .and_then(serde_json::Value::as_i64)
                                    .unwrap_or(-1);
                                let index = params
                                    .get("index")
                                    .and_then(serde_json::Value::as_u64)
                                    .unwrap_or(0) as u32;
                                let done = params
                                    .get("done")
                                    .and_then(serde_json::Value::as_bool)
                                    .unwrap_or(false);
                                let mgr = notif_mcp_manager.clone();
                                let sid = notif.server_id.clone();
                                tokio::spawn(async move {
                                    mgr.handle_stream_chunk(&sid, request_id, index, done)
                                        .await;
                                });
                            }
                            // Fall through to normal notification forwarding
                        }

                        // Child stderr, bridged as a kernel-internal pseudo-notification
                        // → McpServerLog{source:Stderr} for the dashboard Log tab. Must
                        // precede the notifications/cloto.* whitelist below (which would
                        // otherwise re-forward it as a plain McpNotification).
                        // docs/MCP_SERVER_LOGS_DESIGN.md §6.
                        if notif.method == managers::mcp_client::CLOTO_STDERR_LOG_METHOD {
                            let message = managers::mcp_client::stderr_line_from_params(
                                notif.params.as_ref(),
                            );
                            let event_data = cloto_shared::ClotoEventData::McpServerLog {
                                server_id: notif.server_id,
                                source: cloto_shared::McpLogSource::Stderr,
                                level: None,
                                logger: None,
                                message,
                                timestamp: chrono::Utc::now().to_rfc3339(),
                            };
                            let envelope = EnvelopedEvent::system(event_data);
                            if let Err(e) = notif_event_tx.send(envelope).await {
                                tracing::warn!("Failed to forward MCP stderr log: {}", e);
                            }
                            continue;
                        }

                        // Standard MCP logging capability: a server's
                        // `notifications/message` ({level, logger?, data}) →
                        // McpServerLog{source:McpLogging}. Additive to the
                        // mgp.*/cloto.* whitelist below (this method does not
                        // match it, so it would otherwise be dropped).
                        // docs/MCP_SERVER_LOGS_DESIGN.md §7.
                        if notif.method == "notifications/message" {
                            let (level, logger, message) =
                                managers::mcp_client::mcp_log_from_params(notif.params.as_ref());
                            let event_data = cloto_shared::ClotoEventData::McpServerLog {
                                server_id: notif.server_id,
                                source: cloto_shared::McpLogSource::McpLogging,
                                level,
                                logger,
                                message,
                                timestamp: chrono::Utc::now().to_rfc3339(),
                            };
                            let envelope = EnvelopedEvent::system(event_data);
                            if let Err(e) = notif_event_tx.send(envelope).await {
                                tracing::warn!("Failed to forward MCP logging notification: {}", e);
                            }
                            continue;
                        }

                        // Method-based filtering: MGP notifications → event bus, others → log only
                        if notif.method.starts_with("notifications/mgp.")
                            || notif.method.starts_with("notifications/cloto.")
                        {
                            info!(
                                server = %notif.server_id,
                                method = %notif.method,
                                "📨 MCP server notification received"
                            );
                            let event_data = cloto_shared::ClotoEventData::McpNotification {
                                server_id: notif.server_id,
                                method: notif.method,
                                params: notif.params.unwrap_or(serde_json::Value::Null),
                            };
                            let envelope = EnvelopedEvent::system(event_data);
                            if let Err(e) = notif_event_tx.send(envelope).await {
                                tracing::warn!("Failed to forward MCP notification: {}", e);
                            }
                        } else {
                            tracing::debug!(
                                server = %notif.server_id,
                                method = %notif.method,
                                "MCP notification received (not forwarded)"
                            );
                        }
                    }
                }
            }
        });
    }

    // 6c. Cron job scheduler (Layer 2: Autonomous Trigger)
    if config.cron_enabled {
        managers::scheduler::spawn_cron_task(
            pool.clone(),
            event_tx.clone(),
            config.cron_check_interval_secs,
            app_state.shutdown.clone(),
        );
    }

    // 6d. Startup health scan (optional, default: on). Runs the full defender
    // registry — including advisory-feed evaluation (DEFENDER_DESIGN.md §5,
    // "on scan + boot") — in the background, off the critical startup path.
    if config.health_scan_on_startup {
        let scan_pool = pool.clone();
        let scan_report = app_state.last_health_report.clone();
        let scan_data_dir = data_dir.clone();
        let scan_port = config.port;
        tokio::spawn(async move {
            let servers_dir = managers::mcp_venv::resolve_venv_dir()
                .and_then(|v| v.parent().map(std::path::Path::to_path_buf));
            let ctx = defender::checks::CheckCtx {
                pool: Some(scan_pool),
                data_dir: scan_data_dir,
                servers_dir,
                in_kernel: true,
                port: scan_port,
                offline: false,
            };
            let report = defender::checks::run_scan(&ctx).await.report;
            let issue_count = report
                .checks
                .iter()
                .filter(|c| c.status != db::health::HealthStatus::Healthy)
                .count();
            if issue_count > 0 {
                tracing::warn!("⚠️ Startup health scan: {issue_count} issue(s) detected");
            } else {
                tracing::info!("✓ Startup health scan: all clear");
            }
            let mut cached = scan_report.write().await;
            *cached = Some(report);
        });
    }

    // 6e. Internal LLM Proxy (MGP §13.4 — centralized API key management)
    //     Check result in background to avoid blocking HTTP server startup.
    let llm_proxy_rx = managers::llm_proxy::spawn_llm_proxy(
        pool.clone(),
        config.llm_proxy_port,
        config.llm_proxy_token.clone(),
        config.llm_proxy_require_token,
        config.llm_proxy_timeout_secs,
        app_state.shutdown.clone(),
    );
    {
        let proxy_port = config.llm_proxy_port;
        tokio::spawn(async move {
            match tokio::time::timeout(std::time::Duration::from_secs(15), llm_proxy_rx).await {
                Ok(Ok(Ok(()))) => {
                    info!("LLM Proxy ready on port {}", proxy_port);
                }
                Ok(Ok(Err(msg))) => {
                    tracing::warn!(
                        "⚠️  LLM Proxy failed to start: {}. Mind servers will not have LLM access.",
                        msg
                    );
                }
                Ok(Err(_)) => {
                    tracing::warn!("⚠️  LLM Proxy startup channel dropped unexpectedly");
                }
                Err(_) => {
                    tracing::warn!(
                        "⚠️  LLM Proxy startup timed out (15s). Mind servers may not have LLM access."
                    );
                }
            }
        });
    }

    let event_tx_clone = event_tx.clone();
    let processor_clone = processor.clone();
    let shutdown_clone = app_state.shutdown.clone();
    tokio::spawn(async move {
        tokio::select! {
            () = shutdown_clone.notified() => {
                tracing::info!("Event processor shutting down");
            }
            () = processor_clone.process_loop(event_rx, event_tx_clone) => {}
        }
    });

    // 6b. Rate limiter cleanup task (every 10 minutes)
    let rl = rate_limiter.clone();
    let il = install_limiter.clone();
    let shutdown_clone = app_state.shutdown.clone();
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(RATE_LIMITER_CLEANUP_SECS));
        loop {
            tokio::select! {
                () = shutdown_clone.notified() => {
                    tracing::info!("Rate limiter cleanup shutting down");
                    break;
                }
                _ = interval.tick() => {
                    rl.cleanup();
                    il.cleanup();
                }
            }
        }
    });

    // 6e. Revoked keys TTL cleanup task (every 6 hours, bug-158)
    {
        let pool_clone = pool.clone();
        let revoked_keys_clone = app_state.revoked_keys.clone();
        let shutdown_clone = app_state.shutdown.clone();
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(REVOKED_KEYS_CLEANUP_SECS));
            loop {
                tokio::select! {
                    () = shutdown_clone.notified() => {
                        tracing::info!("Revoked keys cleanup shutting down");
                        break;
                    }
                    _ = interval.tick() => {
                        match db::cleanup_revoked_keys(&pool_clone, 90).await {
                            Ok(remaining) => {
                                {
                                    let mut cache = revoked_keys_clone.write().await;
                                    cache.clear();
                                    cache.extend(remaining);
                                }
                                tracing::debug!("Revoked keys cleanup completed");
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "Revoked keys cleanup failed");
                            }
                        }
                    }
                }
            }
        });
    }

    // 7. Web Server

    // Admin endpoints: rate-limited (10 req/s, burst 50)
    let admin_routes = Router::new()
        .route("/health/scan", get(handlers::health::scan_handler))
        .route("/health/repair", post(handlers::health::repair_handler))
        .route("/system/shutdown", post(handlers::shutdown_handler))
        // Same admin-auth class as shutdown, and the same exit path — it just
        // hands the installation to a detached helper on the way out (§7).
        .route(
            "/system/uninstall",
            post(handlers::system_uninstall_handler),
        )
        // The Danger Zone's first gate (§7): read-only enumeration, so the
        // dashboard can show what an uninstall would remove before one is
        // started. Admin-auth because a plan names the credential-bearing
        // paths, not because it changes anything.
        .route(
            "/system/uninstall/plan",
            get(handlers::system_uninstall_plan_handler),
        )
        .route("/plugins/apply", post(handlers::apply_plugin_settings))
        .route("/plugins/{id}/config", post(handlers::update_plugin_config))
        .route(
            "/plugins/{id}/permissions",
            get(handlers::get_plugin_permissions).delete(handlers::revoke_permission_handler),
        )
        .route(
            "/plugins/{id}/permissions/grant",
            post(handlers::grant_permission_handler),
        )
        .route("/agents", post(handlers::create_agent))
        .route(
            "/agents/{id}",
            post(handlers::update_agent).delete(handlers::delete_agent),
        )
        .route("/agents/{id}/power", post(handlers::power_toggle))
        .route(
            "/agents/{id}/avatar",
            get(handlers::get_avatar)
                .post(handlers::upload_avatar)
                .delete(handlers::delete_avatar),
        )
        .route(
            "/agents/{id}/vrm",
            get(handlers::get_vrm)
                .post(handlers::upload_vrm)
                .delete(handlers::delete_vrm),
        )
        .route("/agents/{id}/visemes", post(handlers::generate_visemes))
        // Agent-centric bulk MCP access update (batch replacement of server_grant entries)
        .route(
            "/agents/{id}/mcp-access",
            axum::routing::put(handlers::put_agent_mcp_access),
        )
        .route(
            "/agents/{id}/last-usage",
            get(handlers::get_agent_last_usage),
        )
        // Recall precision (knob 3) — optional memory-capability op,
        // routed to the agent's memory server via the capability dispatcher.
        .route(
            "/agents/{id}/recall-precision",
            get(handlers::get_recall_precision).post(handlers::set_recall_precision),
        )
        .route("/speech/{filename}", get(handlers::serve_speech_file))
        .route("/events/publish", post(handlers::post_event_handler))
        // Cron job management (Layer 2: Autonomous Trigger)
        .route(
            "/cron/jobs",
            get(handlers::list_cron_jobs).post(handlers::create_cron_job),
        )
        .route("/cron/jobs/{id}", delete(handlers::delete_cron_job))
        .route("/cron/jobs/{id}/toggle", post(handlers::toggle_cron_job))
        .route("/cron/jobs/{id}/run", post(handlers::run_cron_job_now))
        // LLM Provider management (MGP §13.4 — centralized key management)
        .route("/llm/providers", get(handlers::list_llm_providers))
        .route(
            "/llm/providers/{id}/key",
            post(handlers::set_llm_provider_key).delete(handlers::delete_llm_provider_key),
        )
        .route(
            "/llm/providers/{id}/model",
            post(handlers::set_llm_provider_model),
        )
        .route(
            "/llm/providers/{id}/models",
            get(handlers::list_provider_models),
        )
        .route(
            "/llm/providers/{id}/context-length",
            post(handlers::set_llm_provider_context_length),
        )
        .route(
            "/llm/providers/{id}/thinking-mode",
            post(handlers::set_llm_provider_thinking_mode),
        )
        .route(
            "/llm/providers/{id}/test",
            post(handlers::test_provider_connection),
        )
        .route(
            "/permissions/{id}/approve",
            post(handlers::approve_permission),
        )
        .route("/permissions/{id}/deny", post(handlers::deny_permission))
        // Command approval endpoints
        .route("/commands/{id}/approve", post(handlers::approve_command))
        .route("/commands/{id}/trust", post(handlers::trust_command))
        .route("/commands/{id}/deny", post(handlers::deny_command))
        // M-08: chat_handler moved here to apply rate limiting
        .route("/chat", post(handlers::chat_handler))
        // Chat persistence endpoints
        .route(
            "/chat/{agent_id}/messages",
            get(handlers::chat::get_messages)
                .post(handlers::chat::post_message)
                .delete(handlers::chat::delete_messages),
        )
        .route(
            "/chat/{agent_id}/messages/{message_id}/retry",
            post(handlers::chat::retry_response),
        )
        .route(
            "/chat/attachments/{attachment_id}",
            get(handlers::chat::get_attachment),
        )
        // MCP dynamic server management
        .route(
            "/mcp/servers",
            get(handlers::list_mcp_servers).post(handlers::create_mcp_server),
        )
        .route(
            "/mcp/servers/{name}",
            axum::routing::delete(handlers::delete_mcp_server),
        )
        // MCP server settings & access control (MCP_SERVER_UI_DESIGN.md §4)
        .route(
            "/mcp/servers/{name}/settings",
            get(handlers::get_mcp_server_settings).put(handlers::update_mcp_server_settings),
        )
        .route(
            "/mcp/servers/{name}/access",
            get(handlers::get_mcp_server_access).put(handlers::put_mcp_server_access),
        )
        // MCP server lifecycle
        .route(
            "/mcp/servers/{name}/restart",
            post(handlers::restart_mcp_server),
        )
        .route(
            "/mcp/servers/{name}/start",
            post(handlers::start_mcp_server),
        )
        .route("/mcp/servers/{name}/stop", post(handlers::stop_mcp_server))
        // Direct tool call for coordinator-pattern servers (MGP §5.6, §19.1)
        .route("/mcp/call", post(handlers::call_mcp_tool))
        // Settings
        .route(
            "/settings/yolo",
            get(handlers::get_yolo_mode).put(handlers::set_yolo_mode),
        )
        .route(
            "/settings/max-cron-generation",
            get(handlers::get_max_cron_generation).put(handlers::set_max_cron_generation),
        )
        // API key invalidation
        .route("/system/invalidate-key", post(handlers::invalidate_api_key))
        .route("/system/regenerate-key", post(handlers::regenerate_api_key))
        // Marketplace (auth required)
        .route("/marketplace/catalog", get(handlers::catalog_handler))
        .route("/marketplace/install", post(handlers::install_handler))
        .route(
            "/marketplace/batch-install",
            post(handlers::batch_install_handler),
        )
        .route(
            "/marketplace/servers/{id}",
            delete(handlers::uninstall_handler),
        );

    // Read endpoints (authenticated, rate-limited — bug-157)
    let api_routes = Router::new()
        .route("/system/version", get(handlers::version_handler))
        .route("/system/health", get(handlers::health_handler))
        // Bootstrap setup (no auth — like health_handler)
        .route("/setup/status", get(handlers::setup::status_handler))
        .route("/setup/progress", get(handlers::setup::progress_handler))
        // Marketplace progress (no auth — SSE stream)
        .route(
            "/marketplace/progress",
            get(handlers::marketplace_progress_handler),
        )
        .route("/events", get(handlers::sse_handler))
        .route("/history", get(handlers::get_history))
        .route("/metrics", get(handlers::get_metrics))
        .route("/memories", get(handlers::get_memories))
        .route(
            "/memories/{id}",
            delete(handlers::delete_memory).put(handlers::update_memory),
        )
        .route("/memories/{id}/lock", post(handlers::lock_memory))
        .route("/memories/{id}/unlock", post(handlers::unlock_memory))
        .route("/episodes", get(handlers::get_episodes))
        .route("/episodes/{id}", delete(handlers::delete_episode))
        .route("/memories/import", post(handlers::import_memories))
        .route("/plugins", get(handlers::get_plugins))
        .route("/plugins/{id}/config", get(handlers::get_plugin_config))
        .route("/agents", get(handlers::get_agents))
        .route(
            "/permissions/pending",
            get(handlers::get_pending_permissions),
        )
        // MCP access control (public/read)
        .route(
            "/mcp/access/by-agent/{agent_id}",
            get(handlers::get_agent_access),
        )
        .merge(admin_routes)
        .layer(axum::middleware::from_fn_with_state(
            app_state.clone(),
            middleware::rate_limit_middleware,
        ))
        .layer(axum::extract::DefaultBodyLimit::max(50 * 1024 * 1024)); // 50MB for VRM uploads

    let app = Router::new()
        .nest("/api", api_routes.with_state(app_state.clone()))
        .route("/api/plugin/{*path}", any(dynamic_proxy_handler))
        .with_state(app_state.clone())
        .fallback(handlers::assets::static_handler)
        .layer(
            CorsLayer::new()
                .allow_origin(config.cors_origins)
                .allow_methods([
                    axum::http::Method::GET,
                    axum::http::Method::POST,
                    axum::http::Method::DELETE,
                    axum::http::Method::PUT,
                ])
                .allow_headers([
                    axum::http::header::CONTENT_TYPE,
                    axum::http::HeaderName::from_static("x-api-key"),
                ]),
        );

    // Build the address from its parts: formatting "{ip}:{port}" would turn an
    // IPv6 bind such as `::1` into `::1:8081`, which is not a socket address.
    let bind_ip: std::net::IpAddr = config.bind_address.parse().map_err(|e| {
        anyhow::anyhow!(
            "BIND_ADDRESS '{}' is not an IP address: {e}",
            config.bind_address
        )
    })?;
    let bind_addr = std::net::SocketAddr::new(bind_ip, config.port);
    let listener = bind_with_retry(bind_addr, 5, std::time::Duration::from_secs(2)).await?;
    // Signal deferred MCP boot that the HTTP server is now ready for callbacks.
    http_ready.notify_waiters();
    info!(
        "🚀 ClotoCore Kernel is listening on http://{}:{} (startup: {:.1}s)",
        config.bind_address,
        config.port,
        kernel_start.elapsed().as_secs_f64()
    );

    // Record who is running this installation, now that it demonstrably is.
    // `clotocore uninstall --execute` reads this and refuses to remove a live
    // install out from under itself (`defender::runlock`).
    defender::runlock::acquire(&app_state.data_dir);

    let shutdown_handle = app_state.shutdown.clone();
    let shutdown_signal = app_state.shutdown.clone();
    let lock_dir = app_state.data_dir.clone();
    let server_task = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            shutdown_signal.notified().await;
            info!("🛑 Graceful shutdown signal received. Stopping server...");
        })
        .await
        .ok();
        // Best-effort: a kill -9 leaves the record behind, which the reader
        // treats as stale once the pid is gone.
        defender::runlock::release(&lock_dir);
    });

    Ok(KernelHandle {
        shutdown: shutdown_handle,
        mcp_manager: app_state.mcp_manager.clone(),
        server_task,
    })
}

/// Initialize the kernel and block until shutdown.
///
/// Convenience wrapper around [`start_kernel`] for standalone CLI usage.
pub async fn run_kernel() -> anyhow::Result<()> {
    let handle = start_kernel().await?;
    // Block until either the HTTP shutdown endpoint fires or the process is
    // asked to stop by its supervisor. A service manager (systemd, launchd,
    // a container runtime) stops a daemon with SIGTERM and has no API key to
    // call `/api/system/shutdown`; without this arm the signal killed the
    // process outright and left every MCP child orphaned (the same leak the
    // HTTP path and the desktop app-exit path already drain against).
    tokio::select! {
        () = handle.shutdown.notified() => {}
        () = stop_signal() => {
            tracing::info!("🛑 Stop signal received from the OS. Draining MCP servers before exit...");
            handle
                .mcp_manager
                .drain_all("kernel shutdown (signal)", 5000, 10)
                .await;
            tracing::info!("👋 Kernel shutting down gracefully.");
            handle.shutdown.notify_waiters();
        }
    }
    // Let the HTTP server finish its graceful shutdown (and release the run
    // lock) before the process exits.
    let _ = handle.server_task.await;
    Ok(())
}

/// Resolve when the process receives a stop request from the OS: SIGTERM or
/// SIGINT on Unix, Ctrl-C / console close on Windows. Never resolves if the
/// signal handlers cannot be installed, so the HTTP shutdown path keeps working.
async fn stop_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    "SIGTERM handler not installed ({e}); stop via /api/system/shutdown"
                );
                std::future::pending::<()>().await;
                unreachable!()
            }
        };
        let mut int = match signal(SignalKind::interrupt()) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("SIGINT handler not installed ({e}); stop via /api/system/shutdown");
                std::future::pending::<()>().await;
                unreachable!()
            }
        };
        tokio::select! {
            _ = term.recv() => {}
            _ = int.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::warn!("Ctrl-C handler not installed ({e}); stop via /api/system/shutdown");
            std::future::pending::<()>().await;
        }
    }
}

/// `CLOTO_ALLOW_UNAUTHENTICATED_HTTP=1` opts out of the refusal to start a
/// keyless kernel on a non-loopback bind. The opt-out exists so an operator
/// who has put another boundary in front of the listener can say so
/// explicitly; it is never the default.
fn unauthenticated_http_allowed() -> bool {
    std::env::var("CLOTO_ALLOW_UNAUTHENTICATED_HTTP")
        .is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
}

/// Bind a TCP listener with retry logic for port conflicts (e.g., previous process
/// still holding the port in CLOSE_WAIT/TIME_WAIT state during `tauri dev` restarts).
async fn bind_with_retry(
    addr: std::net::SocketAddr,
    max_retries: u32,
    delay: std::time::Duration,
) -> anyhow::Result<tokio::net::TcpListener> {
    for attempt in 0..=max_retries {
        let socket = if addr.is_ipv4() {
            tokio::net::TcpSocket::new_v4()?
        } else {
            tokio::net::TcpSocket::new_v6()?
        };
        socket.set_reuseaddr(true)?;
        match socket.bind(addr) {
            Ok(()) => match socket.listen(1024) {
                Ok(listener) => return Ok(listener),
                Err(e) if attempt < max_retries => {
                    tracing::warn!(
                        "Port {} listen failed (attempt {}/{}): {}. Retrying in {:?}...",
                        addr.port(),
                        attempt + 1,
                        max_retries,
                        e,
                        delay
                    );
                    tokio::time::sleep(delay).await;
                }
                Err(e) => return Err(e.into()),
            },
            Err(e) if attempt < max_retries => {
                tracing::warn!(
                    "Port {} bind failed (attempt {}/{}): {}. Retrying in {:?}...",
                    addr.port(),
                    attempt + 1,
                    max_retries,
                    e,
                    delay
                );
                tokio::time::sleep(delay).await;
            }
            Err(e) => return Err(e.into()),
        }
    }
    unreachable!()
}

use axum::extract::State;
use axum::http::Request;
use axum::response::IntoResponse;
use tower::ServiceExt;

async fn dynamic_proxy_handler(
    State(state): State<Arc<AppState>>,
    request: Request<axum::body::Body>,
) -> impl IntoResponse {
    let router = {
        let router_lock = state.dynamic_router.router.read().await;
        router_lock.clone()
    };

    let any_state = state.clone() as Arc<dyn std::any::Any + Send + Sync>;
    router
        .with_state(any_state)
        .oneshot(request)
        .await
        .into_response()
}
