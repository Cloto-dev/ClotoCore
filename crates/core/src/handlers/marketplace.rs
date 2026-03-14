//! Marketplace endpoints for discovering and installing MCP servers
//! from the cloto-mcp-servers registry.

use axum::{
    extract::{Query, State},
    http::HeaderMap,
    response::sse::{Event, Sse},
    Json,
};
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, convert::Infallible, path::PathBuf, sync::Arc, time::Duration};
use tracing::{error, info, warn};

use super::setup::{SetupProgressEvent, detect_python, emit, venv_pip};
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
}

fn default_trust_level() -> String {
    "standard".to_string()
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

// ── Endpoints ───────────────────────────────────────────────────────

/// GET /api/marketplace/catalog — fetch registry and merge with local state.
pub async fn catalog_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<CatalogQuery>,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;

    let registry = fetch_registry(&state, query.force_refresh).await.map_err(|e| {
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
            let mp_record = marketplace_servers.iter().find(|r| r.marketplace_id.as_deref() == Some(&entry.id));
            let installed = mp_record.map(|r| r.is_active).unwrap_or(false);
            let installed_version = mp_record.and_then(|r| r.installed_version.clone());
            let update_available = installed_version
                .as_deref()
                .map(|iv| iv != entry.version)
                .unwrap_or(false);
            let running = running_servers
                .iter()
                .any(|s| s.id == entry.id && matches!(s.status, crate::managers::mcp_types::ServerStatus::Connected));

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
                installed,
                installed_version,
                update_available,
                running,
            }
        })
        .collect();

    let cached_at = {
        let cache = state.marketplace_cache.read().await;
        cache
            .fetched_at
            .map(|t| format!("{:.0}s ago", t.elapsed().as_secs()))
            .unwrap_or_else(|| "just now".to_string())
    };

    super::ok_data(serde_json::json!({
        "servers": entries,
        "cached_at": cached_at,
    }))
}

/// POST /api/marketplace/install — install a server from the marketplace.
pub async fn install_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<InstallRequest>,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;

    // Validate server_id against cached registry
    let registry = {
        let cache = state.marketplace_cache.read().await;
        cache.data.clone()
    };
    let registry = match registry {
        Some(r) => r,
        None => fetch_registry(&state, false).await.map_err(|e| {
            AppError::Internal(anyhow::anyhow!("Registry not available: {e}"))
        })?,
    };

    let entry = registry
        .servers
        .iter()
        .find(|s| s.id == request.server_id)
        .cloned()
        .ok_or_else(|| {
            AppError::Validation(format!("Server '{}' not found in registry", request.server_id))
        })?;

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
        let result =
            run_install(&state_clone, &entry, env_overrides, auto_start).await;
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
    let url =
        "https://raw.githubusercontent.com/Cloto-dev/cloto-mcp-servers/main/registry.json";

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

// ── Install orchestration ───────────────────────────────────────────

async fn run_install(
    state: &AppState,
    entry: &RegistryEntry,
    env_overrides: HashMap<String, String>,
    auto_start: bool,
) -> anyhow::Result<()> {
    let tx = &state.setup_progress_tx;

    // Step 1: Check Python
    emit(tx, SetupProgressEvent::StepStart {
        step: "check_python".into(),
        description: "Checking Python installation".into(),
    });

    let (python_available, _) = detect_python();
    if !python_available {
        emit(tx, SetupProgressEvent::StepError {
            step: "check_python".into(),
            error: "Python 3 is required but not found".into(),
            recoverable: false,
        });
        return Ok(());
    }
    emit(tx, SetupProgressEvent::StepComplete { step: "check_python".into() });

    // Step 2: Download repo tarball
    emit(tx, SetupProgressEvent::StepStart {
        step: "download".into(),
        description: format!("Downloading {} from repository", entry.name),
    });

    let tmp_dir = state.data_dir.join("tmp");
    tokio::fs::create_dir_all(&tmp_dir).await?;
    let archive_path = tmp_dir.join("cloto-mcp-servers-latest.tar.gz");

    let tarball_url =
        "https://api.github.com/repos/Cloto-dev/cloto-mcp-servers/tarball/main";

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
        emit(tx, SetupProgressEvent::StepError {
            step: "download".into(),
            error: format!("GitHub API returned HTTP {}", resp.status()),
            recoverable: true,
        });
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
            emit(tx, SetupProgressEvent::StepProgress {
                step: "download".into(),
                progress,
                detail: format!("{mb_done:.1} / {mb_total:.1} MB"),
            });
        }
    }
    file.flush().await?;

    emit(tx, SetupProgressEvent::StepComplete { step: "download".into() });

    // Step 3: Selective extraction
    emit(tx, SetupProgressEvent::StepStart {
        step: "extract".into(),
        description: format!("Extracting {}", entry.name),
    });

    let servers_dir = resolve_servers_dir(state);
    let directory = entry.directory.clone();
    let needs_common = entry.dependencies.contains(&"common".to_string())
        && !servers_dir.join("common").join("__init__.py").exists();

    let archive_path_clone = archive_path.clone();
    let servers_dir_clone = servers_dir.clone();

    tokio::task::spawn_blocking(move || {
        extract_selective(&archive_path_clone, &servers_dir_clone, &directory, needs_common)
    })
    .await??;

    emit(tx, SetupProgressEvent::StepComplete { step: "extract".into() });

    // Step 4: Install dependencies
    emit(tx, SetupProgressEvent::StepStart {
        step: "install_deps".into(),
        description: format!("Installing {} dependencies", entry.name),
    });

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
            emit(tx, SetupProgressEvent::ServerInstall {
                server_name: "common".into(),
                status: "installing".into(),
            });
            let result = tokio::process::Command::new(&pip_str)
                .args(["install", &common_path.to_string_lossy(), "--quiet"])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output()
                .await;
            match result {
                Ok(output) if output.status.success() => {
                    emit(tx, SetupProgressEvent::ServerInstall {
                        server_name: "common".into(),
                        status: "installed".into(),
                    });
                }
                _ => {
                    warn!("Failed to install common dependency");
                }
            }
        }
    }

    // Install the target server
    let server_path = servers_dir.join(&entry.directory);
    emit(tx, SetupProgressEvent::ServerInstall {
        server_name: entry.name.clone(),
        status: "installing".into(),
    });

    let result = tokio::process::Command::new(&pip_str)
        .args(["install", &server_path.to_string_lossy(), "--quiet"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await;

    match result {
        Ok(output) if output.status.success() => {
            emit(tx, SetupProgressEvent::ServerInstall {
                server_name: entry.name.clone(),
                status: "installed".into(),
            });
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let last_line = stderr.lines().last().unwrap_or("unknown error");
            emit(tx, SetupProgressEvent::StepError {
                step: "install_deps".into(),
                error: format!("pip install failed: {last_line}"),
                recoverable: true,
            });
            return Ok(());
        }
        Err(e) => {
            emit(tx, SetupProgressEvent::StepError {
                step: "install_deps".into(),
                error: format!("Failed to run pip: {e}"),
                recoverable: true,
            });
            return Ok(());
        }
    }

    emit(tx, SetupProgressEvent::StepComplete { step: "install_deps".into() });

    // Step 5: Register in database
    emit(tx, SetupProgressEvent::StepStart {
        step: "finalize".into(),
        description: "Registering server".into(),
    });

    // Build env JSON: merge defaults with overrides
    let mut env_map: HashMap<String, String> = HashMap::new();
    for var in &entry.env_vars {
        if let Some(default) = &var.default {
            env_map.insert(var.key.clone(), default.clone());
        }
    }
    for (k, v) in &env_overrides {
        env_map.insert(k.clone(), v.clone());
    }
    let env_json = serde_json::to_string(&env_map).unwrap_or_else(|_| "{}".to_string());

    // Build command and args
    let venv_python = if cfg!(windows) {
        venv_dir.join("Scripts").join("python.exe")
    } else {
        venv_dir.join("bin").join("python")
    };
    let command = venv_python.to_string_lossy().to_string();
    let server_script = server_path.join("server.py").to_string_lossy().to_string();
    let args_json = serde_json::to_string(&[&server_script]).unwrap_or_else(|_| "[]".to_string());

    crate::db::mcp::save_marketplace_server(
        &state.pool,
        &entry.id,
        &command,
        &args_json,
        Some(&entry.description),
        &env_json,
        &entry.version,
        &entry.id,
    )
    .await?;

    info!("Marketplace server registered: {} v{}", entry.id, entry.version);

    // Optionally start the server
    if auto_start {
        if let Err(e) = state.mcp_manager.start_server(&entry.id).await {
            warn!("Failed to auto-start {}: {e}", entry.id);
        }
    }

    emit(tx, SetupProgressEvent::StepComplete { step: "finalize".into() });

    // Cleanup tarball
    let _ = tokio::fs::remove_file(&archive_path).await;

    emit(tx, SetupProgressEvent::Complete);
    info!("Marketplace install complete: {}", entry.id);

    Ok(())
}

// ── Helpers ─────────────────────────────────────────────────────────

fn resolve_servers_dir(state: &AppState) -> PathBuf {
    crate::managers::mcp_venv::resolve_servers_dir_from_config()
        .unwrap_or_else(|| state.data_dir.join("mcp-servers"))
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
            path_str
                .find("/servers/")
                .map(|i| &path_str[i + 1..]) // "servers/{directory}/..."
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
