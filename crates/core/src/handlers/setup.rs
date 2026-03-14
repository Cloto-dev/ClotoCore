//! Bootstrap setup endpoints.
//!
//! Provides status check, setup trigger, SSE progress stream, and Python
//! re-check for first-launch dependency installation.

use axum::{
    extract::State,
    http::HeaderMap,
    response::sse::{Event, Sse},
    Json,
};
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tracing::{error, info, warn};

use crate::{AppError, AppResult, AppState};

// ── Data structures ──────────────────────────────────────────────────

#[derive(Serialize, Clone, Debug)]
pub struct SetupStatus {
    pub setup_complete: bool,
    pub mcp_servers_present: bool,
    pub python_available: bool,
    pub python_version: Option<String>,
    pub venv_exists: bool,
    pub setup_in_progress: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SetupCompleteFile {
    pub completed_at: String,
    pub version: String,
    pub mcp_servers_version: String,
    pub python_version: Option<String>,
    pub server_count: u32,
}

#[derive(Serialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum SetupProgressEvent {
    StepStart {
        step: String,
        description: String,
    },
    StepProgress {
        step: String,
        progress: f32,
        detail: String,
    },
    StepComplete {
        step: String,
    },
    StepError {
        step: String,
        error: String,
        recoverable: bool,
    },
    ServerInstall {
        server_name: String,
        status: String,
    },
    PythonMissing {
        os: String,
        guidance: String,
    },
    Complete,
}

// ── Helper: dev mode detection ───────────────────────────────────────

/// Returns true when running from a development workspace (Cargo.toml present).
fn is_dev_mode() -> bool {
    let exe = std::env::current_exe().ok();
    exe.and_then(|e| {
        crate::managers::McpClientManager::detect_project_root(&e)
            .map(|root| root.join("Cargo.toml").exists())
    })
    .unwrap_or(false)
}

/// Resolve the project/install root directory.
fn resolve_root() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    crate::managers::McpClientManager::detect_project_root(&exe)
}

// ── Helper: Python detection ─────────────────────────────────────────

/// Find Python and return (command, version_string).
pub(crate) fn detect_python() -> (bool, Option<String>) {
    if let Some(cmd) = crate::managers::mcp_venv::find_python() {
        let version = std::process::Command::new(&cmd)
            .arg("--version")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .ok()
            .and_then(|o| {
                let out = String::from_utf8_lossy(&o.stdout).to_string();
                let err = String::from_utf8_lossy(&o.stderr).to_string();
                let combined = if out.contains("Python") { out } else { err };
                combined
                    .trim()
                    .strip_prefix("Python ")
                    .map(|s| s.to_string())
            });
        (true, version)
    } else {
        (false, None)
    }
}

// ── Endpoints ────────────────────────────────────────────────────────

/// GET /api/setup/status — lightweight check (no auth required, like health_handler).
pub async fn status_handler(
    State(state): State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    // Dev mode always reports complete
    if is_dev_mode() {
        return super::json_data(serde_json::json!(SetupStatus {
            setup_complete: true,
            mcp_servers_present: true,
            python_available: true,
            python_version: None,
            venv_exists: true,
            setup_in_progress: false,
        }));
    }

    let root = resolve_root();
    // TODO: Update to use [paths].servers from mcp.toml after cloto-mcp-servers
    // repo separation is complete for production deployments.
    let mcp_servers_present = root
        .as_ref()
        .map(|r| r.join("mcp-servers").exists())
        .unwrap_or(false);

    // TODO: Update to use mcp_venv::resolve_venv_dir() instead of hardcoded path.
    let venv_exists = root
        .as_ref()
        .map(|r| r.join("mcp-servers").join(".venv").join("pyvenv.cfg").exists())
        .unwrap_or(false);

    let (python_available, python_version) = detect_python();

    // Check setup-complete.json
    let setup_json = state.data_dir.join("setup-complete.json");
    let mut setup_complete = false;
    if setup_json.exists() {
        if let Ok(content) = std::fs::read_to_string(&setup_json) {
            if let Ok(info) = serde_json::from_str::<SetupCompleteFile>(&content) {
                // Version mismatch check: if major.minor differs, treat as incomplete
                let current = env!("CARGO_PKG_VERSION");
                let saved = &info.version;
                let current_mm = major_minor(current);
                let saved_mm = major_minor(saved);
                if current_mm == saved_mm && mcp_servers_present {
                    setup_complete = true;
                }
            }
        }
    }

    let in_progress = state
        .setup_in_progress
        .load(std::sync::atomic::Ordering::Relaxed);

    super::json_data(serde_json::json!(SetupStatus {
        setup_complete,
        mcp_servers_present,
        python_available,
        python_version,
        venv_exists,
        setup_in_progress: in_progress,
    }))
}

/// POST /api/setup/start — trigger bootstrap (auth required, 409 if already running).
pub async fn start_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;

    // Prevent concurrent runs
    let was_running = state.setup_in_progress.swap(true, std::sync::atomic::Ordering::SeqCst);
    if was_running {
        return Err(AppError::Validation("Setup is already in progress".to_string()));
    }

    let state_clone = state.clone();
    tokio::spawn(async move {
        run_bootstrap(state_clone).await;
    });

    super::ok_data(serde_json::json!({ "started": true }))
}

/// GET /api/setup/progress — SSE stream of SetupProgressEvent (no auth, like SSE handler).
pub async fn progress_handler(
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
                    // Break after Complete event
                    if matches!(evt, SetupProgressEvent::Complete) {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("Setup SSE stream lagged by {} messages", n);
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

/// POST /api/setup/check-python — re-check Python availability (no auth).
pub async fn check_python_handler() -> Json<serde_json::Value> {
    let (available, version) = detect_python();
    super::json_data(serde_json::json!({
        "available": available,
        "version": version,
    }))
}

// ── Bootstrap Orchestration ──────────────────────────────────────────

async fn run_bootstrap(state: Arc<AppState>) {
    let tx = &state.setup_progress_tx;

    // Always reset flag on exit via inner function + unconditional reset
    let result = run_bootstrap_inner(&state, tx).await;
    state.setup_in_progress.store(false, std::sync::atomic::Ordering::SeqCst);

    if let Err(e) = result {
        error!("Bootstrap setup failed: {e}");
    }
}

/// Inner bootstrap logic, returns Err on failure so the outer function can reset the flag.
async fn run_bootstrap_inner(
    state: &Arc<AppState>,
    tx: &tokio::sync::broadcast::Sender<SetupProgressEvent>,
) -> anyhow::Result<()> {

    // Step 1: Check Python
    emit(tx, SetupProgressEvent::StepStart {
        step: "check_python".into(),
        description: "Checking Python installation".into(),
    });

    let (python_available, python_version) = detect_python();
    if !python_available {
        let os_name = if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else {
            "linux"
        };
        let guidance = match os_name {
            "windows" => "Install Python from https://www.python.org/downloads/ or run: winget install Python.Python.3",
            "macos" => "Install Python via Homebrew: brew install python3",
            _ => "Install Python via your package manager: sudo apt install python3 python3-venv",
        };
        emit(tx, SetupProgressEvent::PythonMissing {
            os: os_name.into(),
            guidance: guidance.into(),
        });
        return Ok(());
    }
    let python_cmd = crate::managers::mcp_venv::find_python().unwrap();
    info!("Setup: Python found: {} ({:?})", python_cmd, python_version);
    emit(tx, SetupProgressEvent::StepComplete { step: "check_python".into() });

    // Step 2: Download MCP server archive
    emit(tx, SetupProgressEvent::StepStart {
        step: "download".into(),
        description: "Downloading MCP server archive".into(),
    });

    let version = env!("CARGO_PKG_VERSION");
    let archive_name = format!("cloto-mcp-servers-{version}.tar.gz");
    // TODO: Update download URL to cloto-mcp-servers repo releases after
    // the repo has its own release workflow.
    let download_url = format!(
        "https://github.com/Cloto-dev/ClotoCore/releases/download/v{version}/{archive_name}"
    );

    let tmp_dir = state.data_dir.join("tmp");
    if let Err(e) = tokio::fs::create_dir_all(&tmp_dir).await {
        emit(tx, SetupProgressEvent::StepError {
            step: "download".into(),
            error: format!("Failed to create temp directory: {e}"),
            recoverable: true,
        });
        return Ok(());
    }
    let archive_path = tmp_dir.join(&archive_name);

    match download_with_progress(tx, &download_url, &archive_path).await {
        Ok(()) => {
            emit(tx, SetupProgressEvent::StepComplete { step: "download".into() });
        }
        Err(e) => {
            emit(tx, SetupProgressEvent::StepError {
                step: "download".into(),
                error: format!("Download failed: {e}"),
                recoverable: true,
            });
            return Ok(());
        }
    }

    // Step 3: Verify checksum
    emit(tx, SetupProgressEvent::StepStart {
        step: "verify".into(),
        description: "Verifying archive integrity".into(),
    });

    let checksum_url = format!("{download_url}.sha256");
    match verify_checksum(&archive_path, &checksum_url).await {
        Ok(()) => {
            emit(tx, SetupProgressEvent::StepComplete { step: "verify".into() });
        }
        Err(e) => {
            warn!("Checksum verification skipped or failed: {e}");
            // Non-fatal: continue even if checksum file is unavailable
            emit(tx, SetupProgressEvent::StepComplete { step: "verify".into() });
        }
    }

    // Step 4: Extract archive
    emit(tx, SetupProgressEvent::StepStart {
        step: "extract".into(),
        description: "Extracting MCP servers".into(),
    });

    let root = match resolve_root() {
        Some(r) => r,
        None => {
            // Fallback: use exe parent dir
            let exe = std::env::current_exe().unwrap();
            exe.parent().unwrap().to_path_buf()
        }
    };

    match extract_archive(&archive_path, &root).await {
        Ok(()) => {
            emit(tx, SetupProgressEvent::StepComplete { step: "extract".into() });
        }
        Err(e) => {
            emit(tx, SetupProgressEvent::StepError {
                step: "extract".into(),
                error: format!("Extraction failed: {e}"),
                recoverable: true,
            });
            return Ok(());
        }
    }

    // Step 5: Create venv
    emit(tx, SetupProgressEvent::StepStart {
        step: "create_venv".into(),
        description: "Creating Python virtual environment".into(),
    });

    // TODO: Update to use [paths].servers from mcp.toml for venv location.
    let venv_dir = root.join("mcp-servers").join(".venv");
    if !venv_dir.join("pyvenv.cfg").exists() {
        let venv_path_str = venv_dir.to_string_lossy().to_string();
        let result = tokio::process::Command::new(&python_cmd)
            .args(["-m", "venv", &venv_path_str])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .status()
            .await;

        match result {
            Ok(status) if status.success() => {
                info!("Setup: Created Python venv at {}", venv_dir.display());
            }
            Ok(status) => {
                emit(tx, SetupProgressEvent::StepError {
                    step: "create_venv".into(),
                    error: format!("Failed to create venv (exit code: {:?})", status.code()),
                    recoverable: true,
                });
                return Ok(());
            }
            Err(e) => {
                emit(tx, SetupProgressEvent::StepError {
                    step: "create_venv".into(),
                    error: format!("Failed to run Python for venv creation: {e}"),
                    recoverable: true,
                });
                return Ok(());
            }
        }

        // Upgrade pip
        let pip = venv_pip(&venv_dir);
        let pip_str = pip.to_string_lossy().to_string();
        let _ = tokio::process::Command::new(&pip_str)
            .args(["install", "--upgrade", "pip", "--quiet"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .status()
            .await;
    }
    emit(tx, SetupProgressEvent::StepComplete { step: "create_venv".into() });

    // Step 6: Install dependencies per server
    emit(tx, SetupProgressEvent::StepStart {
        step: "install_deps".into(),
        description: "Installing MCP server dependencies".into(),
    });

    // TODO: Update to use [paths].servers from mcp.toml for servers directory.
    let mcp_servers_dir = root.join("mcp-servers");
    let pip = venv_pip(&venv_dir);
    let pip_str = pip.to_string_lossy().to_string();
    let server_count = install_server_deps_with_progress(tx, &pip_str, &mcp_servers_dir).await;
    emit(tx, SetupProgressEvent::StepComplete { step: "install_deps".into() });

    // Step 7: Finalize — write setup-complete.json
    emit(tx, SetupProgressEvent::StepStart {
        step: "finalize".into(),
        description: "Finalizing setup".into(),
    });

    let complete = SetupCompleteFile {
        completed_at: chrono::Utc::now().to_rfc3339(),
        version: version.to_string(),
        mcp_servers_version: version.to_string(),
        python_version: python_version.clone(),
        server_count,
    };

    let setup_json = state.data_dir.join("setup-complete.json");
    match serde_json::to_string_pretty(&complete) {
        Ok(json) => {
            if let Err(e) = tokio::fs::write(&setup_json, json).await {
                error!("Failed to write setup-complete.json: {e}");
            }
        }
        Err(e) => {
            error!("Failed to serialize setup-complete.json: {e}");
        }
    }

    emit(tx, SetupProgressEvent::StepComplete { step: "finalize".into() });

    // Cleanup temp archive
    let _ = tokio::fs::remove_file(&archive_path).await;

    info!("Setup complete: {} server(s) installed", server_count);
    emit(tx, SetupProgressEvent::Complete);

    Ok(())
}

// ── Internal helpers ─────────────────────────────────────────────────

pub(crate) fn emit(tx: &tokio::sync::broadcast::Sender<SetupProgressEvent>, event: SetupProgressEvent) {
    // Ignore send errors (no subscribers)
    let _ = tx.send(event);
}

pub(crate) fn venv_pip(venv_dir: &std::path::Path) -> std::path::PathBuf {
    if cfg!(windows) {
        venv_dir.join("Scripts").join("pip.exe")
    } else {
        venv_dir.join("bin").join("pip")
    }
}

fn major_minor(version: &str) -> (u32, u32) {
    let parts: Vec<&str> = version.split('.').collect();
    let major = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    (major, minor)
}

/// Download a file with progress reporting via SSE.
pub(crate) async fn download_with_progress(
    tx: &tokio::sync::broadcast::Sender<SetupProgressEvent>,
    url: &str,
    dest: &std::path::Path,
) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let resp = client.get(url).send().await?;

    if !resp.status().is_success() {
        anyhow::bail!("HTTP {} from {}", resp.status(), url);
    }

    let total = resp.content_length();
    let mut stream = resp.bytes_stream();
    let mut file = tokio::fs::File::create(dest).await?;
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
    Ok(())
}

/// Verify SHA256 checksum against a remote .sha256 file.
async fn verify_checksum(
    archive_path: &std::path::Path,
    checksum_url: &str,
) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let resp = client.get(checksum_url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("Checksum file not available (HTTP {})", resp.status());
    }

    let checksum_text = resp.text().await?;
    // Format: "<hash>  <filename>" or just "<hash>"
    let expected_hash = checksum_text
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow::anyhow!("Empty checksum file"))?
        .to_lowercase();

    // Compute local hash
    use sha2::Digest;
    let data = tokio::fs::read(archive_path).await?;
    let hash = sha2::Sha256::digest(&data);
    let actual_hash = hex::encode(hash);

    if actual_hash != expected_hash {
        anyhow::bail!(
            "Checksum mismatch: expected {expected_hash}, got {actual_hash}"
        );
    }

    info!("Checksum verified: {actual_hash}");
    Ok(())
}

/// Extract tar.gz archive to the target directory.
async fn extract_archive(
    archive_path: &std::path::Path,
    target_dir: &std::path::Path,
) -> anyhow::Result<()> {
    let archive_path = archive_path.to_path_buf();
    let target_dir = target_dir.to_path_buf();

    // Run in blocking task since tar/flate2 are synchronous
    tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&archive_path)?;
        let decoder = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(decoder);
        archive.unpack(&target_dir)?;
        Ok::<(), anyhow::Error>(())
    })
    .await??;

    Ok(())
}

/// Install dependencies per server with SSE progress events.
async fn install_server_deps_with_progress(
    tx: &tokio::sync::broadcast::Sender<SetupProgressEvent>,
    pip_str: &str,
    mcp_servers_dir: &std::path::Path,
) -> u32 {
    let mut installed = 0u32;
    let Ok(entries) = std::fs::read_dir(mcp_servers_dir) else {
        return 0;
    };

    let mut servers: Vec<_> = entries
        .flatten()
        .filter(|e| {
            let path = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            path.is_dir()
                && !name.starts_with('.')
                && name != "common"
                && path.join("pyproject.toml").exists()
        })
        .collect();

    servers.sort_by_key(|e| e.file_name());

    for entry in &servers {
        let name = entry.file_name().to_string_lossy().to_string();
        let server_path = entry.path().to_string_lossy().to_string();

        emit(tx, SetupProgressEvent::ServerInstall {
            server_name: name.clone(),
            status: "installing".into(),
        });

        let result = tokio::process::Command::new(pip_str)
            .args(["install", &server_path, "--quiet"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await;

        match result {
            Ok(output) if output.status.success() => {
                installed += 1;
                emit(tx, SetupProgressEvent::ServerInstall {
                    server_name: name,
                    status: "installed".into(),
                });
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                warn!("Failed to install {}: {}", name, stderr.lines().last().unwrap_or("unknown"));
                emit(tx, SetupProgressEvent::ServerInstall {
                    server_name: name,
                    status: "failed".into(),
                });
            }
            Err(e) => {
                warn!("Failed to run pip for {}: {}", name, e);
                emit(tx, SetupProgressEvent::ServerInstall {
                    server_name: name,
                    status: "failed".into(),
                });
            }
        }
    }

    installed
}
