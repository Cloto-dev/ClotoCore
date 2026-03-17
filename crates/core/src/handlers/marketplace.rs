//! Marketplace endpoints for discovering and installing MCP servers
//! from the cloto-mcp-servers registry.

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

use super::setup::{detect_python, emit, venv_pip, SetupProgressEvent};
use crate::{AppError, AppResult, AppState};

// ── Registry types ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Registry {
    pub schema_version: u32,
    pub updated_at: String,
    pub servers: Vec<RegistryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub version: String,
    #[serde(default)]
    pub directory: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub env_vars: Vec<EnvVarDef>,
    #[serde(default)]
    pub optional_env_vars: Vec<EnvVarDef>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_trust_level")]
    pub trust_level: String,
    #[serde(default)]
    pub auto_restart: bool,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default = "default_runtime")]
    pub runtime: String,
    #[serde(default)]
    pub bin_name: Option<String>,
}

fn default_trust_level() -> String {
    "standard".to_string()
}

fn default_runtime() -> String {
    "python".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvVarDef {
    pub key: String,
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

// ── Catalog cache ───────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct CatalogCache {
    pub data: Option<Registry>,
    pub fetched_at: Option<tokio::time::Instant>,
}

const CACHE_TTL: Duration = Duration::from_secs(3600); // 1 hour

// ── Response types ──────────────────────────────────────────────────

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

    let registry = fetch_registry(&state, query.force_refresh)
        .await
        .map_err(|e| {
            AppError::Internal(anyhow::anyhow!("Failed to fetch marketplace catalog: {e}"))
        })?;

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
            let installed = mp_record.is_some_and(|r| r.is_active);
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
        "cached_at": cached_at,
    }))
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
            .map_err(|e| AppError::Internal(anyhow::anyhow!("Registry not available: {e}")))?,
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

    // Reject if server is already marketplace-installed (dynamic).
    // Config-loaded servers (from mcp.toml) are allowed — marketplace install
    // builds/registers independently and the config entry is effectively replaced.
    let running_servers = state.mcp_manager.list_servers().await;
    if running_servers.iter().any(|s| {
        s.id == request.server_id && s.source == crate::managers::mcp_types::ServerSource::Dynamic
    }) {
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
    let env_overrides = request.env.unwrap_or_default();
    let auto_start = request.auto_start.unwrap_or(true);

    tokio::spawn(async move {
        let result = run_install(&state_clone, &entry, env_overrides, auto_start).await;
        state_clone
            .setup_in_progress
            .store(false, std::sync::atomic::Ordering::SeqCst);
        if let Err(e) = result {
            error!("Marketplace install failed for {}: {e}", entry.id);
        }
    });

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

    // Disconnect from manager (stops process + removes from memory).
    // remove_dynamic_server() rejects config-loaded servers but allows marketplace.
    // It also calls deactivate_mcp_server() internally, which is fine —
    // we'll hard-delete the DB record right after.
    if let Err(e) = state.mcp_manager.remove_dynamic_server(&server_id).await {
        let msg = e.to_string();
        if msg.contains("config-loaded") {
            return Err(AppError::Validation(
                "Cannot uninstall a config-loaded server. Edit mcp.toml instead.".to_string(),
            ));
        }
        // Server might already be stopped/disconnected — log and continue
        warn!("remove_dynamic_server warning for {}: {}", server_id, msg);
    }

    // Hard-delete from DB (mcp_servers + mcp_access_control)
    if let Err(e) = crate::db::mcp::delete_marketplace_server(&state.pool, &server_id).await {
        warn!("DB cleanup for {}: {}", server_id, e);
        // Don't fail — the server is already disconnected
    }

    // Delete server files from disk.
    // Resolve directory name from catalog cache, fallback to id-based convention.
    let directory = {
        let cache = state.marketplace_cache.read().await;
        cache.data.as_ref().and_then(|reg| {
            reg.servers
                .iter()
                .find(|s| s.id == server_id)
                .map(|s| s.directory.clone())
        })
    }
    .unwrap_or_else(|| server_id.replace('.', "-"));

    let server_dir = state.data_dir.join("mcp-servers").join(&directory);
    if server_dir.is_dir() {
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
        }
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
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Registry not available: {e}")))?;

    let mut entries = Vec::new();
    for id in &request.server_ids {
        match registry.servers.iter().find(|s| &s.id == id) {
            Some(entry) => entries.push(entry.clone()),
            None => {
                return Err(AppError::Validation(format!(
                    "Server '{}' not found in registry",
                    id
                )));
            }
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

    tokio::spawn(async move {
        let result = run_batch_install(&state_clone, &entries, auto_start).await;
        state_clone
            .setup_in_progress
            .store(false, std::sync::atomic::Ordering::SeqCst);
        if let Err(e) = result {
            error!("Batch install failed: {e}");
        }
    });

    super::ok_data(serde_json::json!({
        "started": true,
        "server_ids": request.server_ids,
    }))
}

// ── Registry fetch with cache ───────────────────────────────────────

async fn fetch_registry(state: &AppState, force_refresh: bool) -> anyhow::Result<Registry> {
    // Check cache first
    if !force_refresh {
        let cache = state.marketplace_cache.read().await;
        if let (Some(data), Some(fetched_at)) = (&cache.data, cache.fetched_at) {
            if fetched_at.elapsed() < CACHE_TTL {
                return Ok(data.clone());
            }
        }
    }

    info!("Fetching marketplace registry from GitHub...");
    let url = "https://raw.githubusercontent.com/Cloto-dev/cloto-mcp-servers/master/registry.json";

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;
    let resp = client.get(url).send().await?;

    if !resp.status().is_success() {
        // Fall back to cache if available
        let cache = state.marketplace_cache.read().await;
        if let Some(data) = &cache.data {
            warn!(
                "GitHub API returned {}, using cached registry",
                resp.status()
            );
            return Ok(data.clone());
        }
        anyhow::bail!("Failed to fetch registry: HTTP {}", resp.status());
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

    Ok(registry)
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

    let status = child.wait().await?;
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

async fn run_install(
    state: &AppState,
    entry: &RegistryEntry,
    env_overrides: HashMap<String, String>,
    auto_start: bool,
) -> anyhow::Result<()> {
    let tx = &state.setup_progress_tx;
    let is_rust = entry.runtime == "rust";

    // Step 1: Check toolchain
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
            return Ok(());
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
                step: "check_python".into(),
                description: "Checking Python installation".into(),
            },
        );
        let (python_available, _) = detect_python();
        if !python_available {
            emit(
                tx,
                SetupProgressEvent::StepError {
                    step: "check_python".into(),
                    error: "Python 3 is required but not found".into(),
                    recoverable: false,
                },
            );
            return Ok(());
        }
        emit(
            tx,
            SetupProgressEvent::StepComplete {
                step: "check_python".into(),
            },
        );
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

    let tarball_url = "https://api.github.com/repos/Cloto-dev/cloto-mcp-servers/tarball/master";

    // Download with custom headers for GitHub API
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
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
    let needs_common = !is_rust
        && entry.dependencies.contains(&"common".to_string())
        && !servers_dir.join("common").join("__init__.py").exists();

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

    // Step 4: Install dependencies / build
    let server_path = servers_dir.join(&entry.directory);

    let (command, args, venv_dir) = if is_rust {
        // ── Rust server: cargo build ──
        // Verify extraction produced a valid Cargo project
        if !server_path.join("Cargo.toml").exists() {
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
        info!(
            "Cargo.toml found, starting build in {}",
            server_path.display()
        );

        if !cargo_build_server(tx, &server_path, &entry.name).await? {
            return Ok(());
        }

        let bin_path = rust_binary_path(&server_path, entry);
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

        (
            bin_path.to_string_lossy().to_string(),
            vec![],
            servers_dir.join(".venv"), // unused but needed for type consistency
        )
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

        // Ensure venv exists
        if !venv_dir.join("pyvenv.cfg").exists() {
            if let Some(python_cmd) = crate::managers::mcp_venv::find_python() {
                let venv_path_str = venv_dir.to_string_lossy().to_string();
                let _ = tokio::process::Command::new(&python_cmd)
                    .args(["-m", "venv", &venv_path_str])
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .status()
                    .await;
            }
        }

        let pip = venv_pip(&venv_dir);
        let pip_str = pip.to_string_lossy().to_string();

        // Install common first if needed
        if needs_common {
            let common_path = servers_dir.join("common");
            if common_path.join("pyproject.toml").exists() {
                emit(
                    tx,
                    SetupProgressEvent::ServerInstall {
                        server_name: "common".into(),
                        status: "installing".into(),
                    },
                );
                let result = tokio::process::Command::new(&pip_str)
                    .args(["install", &common_path.to_string_lossy(), "--quiet"])
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .output()
                    .await;
                match result {
                    Ok(output) if output.status.success() => {
                        emit(
                            tx,
                            SetupProgressEvent::ServerInstall {
                                server_name: "common".into(),
                                status: "installed".into(),
                            },
                        );
                    }
                    _ => {
                        warn!("Failed to install common dependency");
                    }
                }
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

        let result = tokio::process::Command::new(&pip_str)
            .args(["install", &server_path.to_string_lossy(), "--quiet"])
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
                        error: format!("pip install failed: {last_line}"),
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
                        error: format!("Failed to run pip: {e}"),
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

        let venv_python = if cfg!(windows) {
            venv_dir.join("Scripts").join("python.exe")
        } else {
            venv_dir.join("bin").join("python")
        };
        let server_script = server_path.join("server.py").to_string_lossy().to_string();
        (
            venv_python.to_string_lossy().to_string(),
            vec![server_script],
            venv_dir,
        )
    };

    // Step 5: Register and start via add_dynamic_server()
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
            env_map.insert(var.key.clone(), default.clone());
        }
    }
    for (k, v) in &env_overrides {
        env_map.insert(k.clone(), v.clone());
    }
    let _ = &venv_dir; // suppress unused warning

    // Use add_dynamic_server() for proper lifecycle integration:
    // creates ServerConfig → connect_server() (spawn + register) → save to DB
    match state
        .mcp_manager
        .add_dynamic_server(
            entry.id.clone(),
            command,
            args,
            None,
            Some(entry.description.clone()),
            None,
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
    if let Err(e) =
        crate::db::mcp::set_marketplace_fields(&state.pool, &entry.id, &entry.version, &entry.id)
            .await
    {
        warn!("Failed to set marketplace fields: {e}");
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

    // Cleanup tarball
    let _ = tokio::fs::remove_file(&archive_path).await;

    emit(tx, SetupProgressEvent::Complete);
    info!("Marketplace install complete: {}", entry.id);

    Ok(())
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Marketplace servers are always installed to {data_dir}/mcp-servers/,
/// separate from config-loaded servers in [paths].servers.
fn resolve_servers_dir(state: &AppState) -> PathBuf {
    state.data_dir.join("mcp-servers")
}

/// Extract specific directories from a GitHub tarball.
/// GitHub tarballs have a prefix like `Cloto-dev-cloto-mcp-servers-{sha}/`.
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

async fn run_batch_install(
    state: &AppState,
    entries: &[RegistryEntry],
    auto_start: bool,
) -> anyhow::Result<()> {
    let tx = &state.setup_progress_tx;
    let has_python_servers = entries.iter().any(|e| e.runtime != "rust");
    let has_rust_servers = entries.iter().any(|e| e.runtime == "rust");

    // Step 1: Check toolchains
    if has_python_servers {
        emit(
            tx,
            SetupProgressEvent::StepStart {
                step: "check_python".into(),
                description: "Checking Python installation".into(),
            },
        );
        let (python_available, _) = detect_python();
        if !python_available {
            emit(
                tx,
                SetupProgressEvent::StepError {
                    step: "check_python".into(),
                    error: "Python 3 is required but not found".into(),
                    recoverable: false,
                },
            );
            return Ok(());
        }
        emit(
            tx,
            SetupProgressEvent::StepComplete {
                step: "check_python".into(),
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

    let tarball_url = "https://api.github.com/repos/Cloto-dev/cloto-mcp-servers/tarball/master";

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
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
        .any(|e| e.runtime != "rust" && e.dependencies.contains(&"common".to_string()))
        && !servers_dir.join("common").join("__init__.py").exists();

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

    let venv_dir =
        crate::managers::mcp_venv::resolve_venv_dir().unwrap_or_else(|| servers_dir.join(".venv"));

    let pip_str = if has_python_servers {
        if !venv_dir.join("pyvenv.cfg").exists() {
            if let Some(python_cmd) = crate::managers::mcp_venv::find_python() {
                let venv_path_str = venv_dir.to_string_lossy().to_string();
                let _ = tokio::process::Command::new(&python_cmd)
                    .args(["-m", "venv", &venv_path_str])
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .status()
                    .await;
            }
        }
        let pip = venv_pip(&venv_dir);
        pip.to_string_lossy().to_string()
    } else {
        String::new()
    };

    // Install common once if needed
    if needs_common {
        let common_path = servers_dir.join("common");
        if common_path.join("pyproject.toml").exists() {
            emit(
                tx,
                SetupProgressEvent::ServerInstall {
                    server_name: "common".into(),
                    status: "installing".into(),
                },
            );
            let result = tokio::process::Command::new(&pip_str)
                .args(["install", &common_path.to_string_lossy(), "--quiet"])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output()
                .await;
            match result {
                Ok(output) if output.status.success() => {
                    emit(
                        tx,
                        SetupProgressEvent::ServerInstall {
                            server_name: "common".into(),
                            status: "installed".into(),
                        },
                    );
                }
                _ => warn!("Failed to install common dependency"),
            }
        }
    }

    // Install each server
    let running_servers = state.mcp_manager.list_servers().await;

    for entry in entries {
        // Skip already-installed marketplace servers (Dynamic source).
        // Config-loaded servers are allowed to be re-installed via marketplace.
        if running_servers.iter().any(|s| {
            s.id == entry.id && s.source == crate::managers::mcp_types::ServerSource::Dynamic
        }) {
            emit(
                tx,
                SetupProgressEvent::ServerInstall {
                    server_name: entry.name.clone(),
                    status: "skipped".into(),
                },
            );
            info!("Batch install: {} already running, skipped", entry.id);
            continue;
        }

        let server_path = servers_dir.join(&entry.directory);
        let is_rust = entry.runtime == "rust";

        emit(
            tx,
            SetupProgressEvent::ServerInstall {
                server_name: entry.name.clone(),
                status: "installing".into(),
            },
        );

        // Build or install depending on runtime
        let (command, args) = if is_rust {
            // Rust server: cargo build --release
            match cargo_build_server(tx, &server_path, &entry.name).await {
                Ok(true) => {}
                Ok(false) => {
                    emit(
                        tx,
                        SetupProgressEvent::ServerInstall {
                            server_name: entry.name.clone(),
                            status: "failed".into(),
                        },
                    );
                    continue;
                }
                Err(e) => {
                    warn!("cargo build failed for {}: {e}", entry.id);
                    emit(
                        tx,
                        SetupProgressEvent::ServerInstall {
                            server_name: entry.name.clone(),
                            status: "failed".into(),
                        },
                    );
                    continue;
                }
            }
            let bin_path = rust_binary_path(&server_path, entry);
            (bin_path.to_string_lossy().to_string(), vec![])
        } else {
            // Python server: pip install
            let result = tokio::process::Command::new(&pip_str)
                .args(["install", &server_path.to_string_lossy(), "--quiet"])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output()
                .await;

            match result {
                Ok(output) if output.status.success() => {}
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    warn!(
                        "pip install failed for {}: {}",
                        entry.id,
                        stderr.lines().last().unwrap_or("")
                    );
                    emit(
                        tx,
                        SetupProgressEvent::ServerInstall {
                            server_name: entry.name.clone(),
                            status: "failed".into(),
                        },
                    );
                    continue;
                }
                Err(e) => {
                    warn!("Failed to run pip for {}: {}", entry.id, e);
                    emit(
                        tx,
                        SetupProgressEvent::ServerInstall {
                            server_name: entry.name.clone(),
                            status: "failed".into(),
                        },
                    );
                    continue;
                }
            }

            let venv_python = if cfg!(windows) {
                venv_dir.join("Scripts").join("python.exe")
            } else {
                venv_dir.join("bin").join("python")
            };
            let server_script = server_path.join("server.py").to_string_lossy().to_string();
            (
                venv_python.to_string_lossy().to_string(),
                vec![server_script],
            )
        };

        // Register via add_dynamic_server
        let mut env_map: HashMap<String, String> = HashMap::new();
        for var in &entry.env_vars {
            if let Some(default) = &var.default {
                env_map.insert(var.key.clone(), default.clone());
            }
        }

        match state
            .mcp_manager
            .add_dynamic_server(
                entry.id.clone(),
                command,
                args,
                None,
                Some(entry.description.clone()),
                None,
                env_map,
            )
            .await
        {
            Ok(tools) => {
                info!("Batch: {} connected ({} tools)", entry.id, tools.len());
            }
            Err(e) => {
                warn!("Batch: {} registered but failed to connect: {e}", entry.id);
            }
        }

        if let Err(e) = crate::db::mcp::set_marketplace_fields(
            &state.pool,
            &entry.id,
            &entry.version,
            &entry.id,
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
                status: "installed".into(),
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
    let setup_json = state.data_dir.join("setup-complete.json");
    if let Ok(json) = serde_json::to_string_pretty(&complete) {
        let _ = tokio::fs::write(&setup_json, json).await;
    }

    // Cleanup
    let _ = tokio::fs::remove_file(&archive_path).await;

    emit(tx, SetupProgressEvent::Complete);
    info!("Batch install complete: {} server(s)", entries.len());

    Ok(())
}
