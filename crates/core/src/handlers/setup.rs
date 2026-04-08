//! Bootstrap setup endpoints.
//!
//! Provides status check, setup trigger, and SSE progress stream
//! for first-launch dependency installation.
//! Uses `uv` for Python venv creation and dependency management.

use crate::AppState;
use axum::{
    extract::State,
    response::sse::{Event, Sse},
    Json,
};
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, sync::Arc, time::Duration};

// ── Data structures ──────────────────────────────────────────────────

#[allow(clippy::struct_excessive_bools)]
#[derive(Serialize, Clone, Debug)]
pub struct SetupStatus {
    pub setup_complete: bool,
    pub mcp_servers_present: bool,
    pub uv_available: bool,
    pub venv_exists: bool,
    pub setup_in_progress: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SetupCompleteFile {
    pub completed_at: String,
    pub version: String,
    pub mcp_servers_version: String,
    #[serde(default)]
    pub uv_version: Option<String>,
    pub server_count: u32,
    // Legacy field for backward compatibility (ignored on read)
    #[serde(default, skip_serializing)]
    pub python_version: Option<String>,
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

// ── Endpoints ────────────────────────────────────────────────────────

/// GET /api/setup/status — lightweight check (no auth required, like health_handler).
pub async fn status_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    // Dev mode always reports complete
    if is_dev_mode() {
        return super::json_data(serde_json::json!(SetupStatus {
            setup_complete: true,
            mcp_servers_present: true,
            uv_available: true,
            venv_exists: true,
            setup_in_progress: false,
        }));
    }

    let root = resolve_root();
    let mcp_servers_present = root
        .as_ref()
        .is_some_and(|r| r.join("mcp-servers").exists());

    let venv_exists = root.as_ref().is_some_and(|r| {
        r.join("mcp-servers")
            .join(".venv")
            .join("pyvenv.cfg")
            .exists()
    });

    let uv_available = crate::managers::mcp_venv::uv_bin(&state.data_dir).exists();

    // Check setup-complete.json
    let setup_json = state.data_dir.join("setup-complete.json");
    let mut setup_complete = false;
    if setup_json.exists() {
        if let Ok(content) = std::fs::read_to_string(&setup_json) {
            if let Ok(info) = serde_json::from_str::<SetupCompleteFile>(&content) {
                // Exact version match: any version change triggers re-setup
                let current = env!("CARGO_PKG_VERSION");
                if current == info.version && mcp_servers_present && venv_exists {
                    setup_complete = true;
                }
            }
        }
    }

    // Fallback: if JSON missing but servers + DB agents exist, treat as complete (bug-378)
    if !setup_complete && mcp_servers_present && venv_exists {
        let agent_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agents")
            .fetch_one(&state.pool)
            .await
            .unwrap_or(0);
        if agent_count > 0 {
            setup_complete = true;
        }
    }

    let in_progress = state
        .setup_in_progress
        .load(std::sync::atomic::Ordering::Relaxed);

    super::json_data(serde_json::json!(SetupStatus {
        setup_complete,
        mcp_servers_present,
        uv_available,
        venv_exists,
        setup_in_progress: in_progress,
    }))
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

// ── Internal helpers ─────────────────────────────────────────────────

pub(crate) fn emit(
    tx: &tokio::sync::broadcast::Sender<SetupProgressEvent>,
    event: SetupProgressEvent,
) {
    // Ignore send errors (no subscribers)
    let _ = tx.send(event);
}

/// Pinned uv version. Update intentionally after testing.
const UV_VERSION: &str = "0.11.3";

/// Get the platform-specific uv download URL and archive extension.
fn uv_download_target() -> (&'static str, &'static str) {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        ("uv-x86_64-pc-windows-msvc", "zip")
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        ("uv-x86_64-unknown-linux-gnu", "tar.gz")
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        ("uv-aarch64-unknown-linux-gnu", "tar.gz")
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        ("uv-x86_64-apple-darwin", "tar.gz")
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        ("uv-aarch64-apple-darwin", "tar.gz")
    }
}

/// Download and extract the uv binary to `{data_dir}/bin/`.
pub(crate) async fn download_uv(
    tx: &tokio::sync::broadcast::Sender<SetupProgressEvent>,
    data_dir: &std::path::Path,
) -> anyhow::Result<()> {
    let (target, ext) = uv_download_target();
    let url =
        format!("https://github.com/astral-sh/uv/releases/download/{UV_VERSION}/{target}.{ext}");

    let bin_dir = data_dir.join("bin");
    tokio::fs::create_dir_all(&bin_dir).await?;

    let tmp_dir = data_dir.join("tmp");
    tokio::fs::create_dir_all(&tmp_dir).await?;
    let archive_path = tmp_dir.join(format!("{target}.{ext}"));

    // Download
    emit(
        tx,
        SetupProgressEvent::StepProgress {
            step: "download_uv".into(),
            progress: -1.0,
            detail: "Downloading uv...".into(),
        },
    );
    download_with_progress(tx, &url, &archive_path).await?;

    // Extract uv binary
    emit(
        tx,
        SetupProgressEvent::StepProgress {
            step: "download_uv".into(),
            progress: -1.0,
            detail: "Extracting uv...".into(),
        },
    );

    let archive_clone = archive_path.clone();
    let bin_dir_clone = bin_dir.clone();
    let target_str = target.to_string();

    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        if target_str.contains("windows") {
            // ZIP archive
            let file = std::fs::File::open(&archive_clone)?;
            let mut archive = zip::ZipArchive::new(file)?;
            for i in 0..archive.len() {
                let mut entry = archive.by_index(i)?;
                let name = entry.name().to_string();
                if name.ends_with("uv.exe") {
                    let dest = bin_dir_clone.join("uv.exe");
                    let mut out = std::fs::File::create(&dest)?;
                    std::io::copy(&mut entry, &mut out)?;
                    break;
                }
            }
        } else {
            // tar.gz archive
            let file = std::fs::File::open(&archive_clone)?;
            let decoder = flate2::read::GzDecoder::new(file);
            let mut archive = tar::Archive::new(decoder);
            for entry in archive.entries()? {
                let mut entry = entry?;
                let path = entry.path()?.to_path_buf();
                if path.file_name().is_some_and(|n| n == "uv") {
                    let dest = bin_dir_clone.join("uv");
                    let mut out = std::fs::File::create(&dest)?;
                    std::io::copy(&mut entry, &mut out)?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))?;
                    }
                    break;
                }
            }
        }
        Ok(())
    })
    .await??;

    // Cleanup archive
    let _ = tokio::fs::remove_file(&archive_path).await;

    // Verify extraction
    let uv = crate::managers::mcp_venv::uv_bin(data_dir);
    if !uv.exists() {
        anyhow::bail!("uv binary not found after extraction at {}", uv.display());
    }

    Ok(())
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
    Ok(())
}

/// Spawn a single `uv pip install` with live stderr streaming and optional log file.
/// uv outputs progress to stderr (Resolved, Prepared, Installed, Downloading, etc.)
pub(crate) async fn spawn_uv_streaming(
    tx: &tokio::sync::broadcast::Sender<SetupProgressEvent>,
    uv_str: &str,
    venv_python_str: &str,
    server_name: &str,
    server_path: &str,
    log_file: Option<&std::path::Path>,
) -> Result<(), String> {
    let mut cmd = tokio::process::Command::new(uv_str);
    cmd.args([
        "pip",
        "install",
        "--no-progress",
        "--python",
        venv_python_str,
        server_path,
    ])
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped());

    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

    let mut child = cmd.spawn().map_err(|e| format!("failed to run uv: {e}"))?;

    // uv outputs progress to stderr
    let tx_clone = tx.clone();
    let srv_name = server_name.to_owned();
    let log_path = log_file.map(std::path::Path::to_path_buf);
    let stderr_stream = child.stderr.take().map(|stderr| {
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut lines = BufReader::new(stderr).lines();
            let mut last: Vec<String> = Vec::new();

            // Open log file in append mode if requested
            let mut log_writer = if let Some(ref lp) = log_path {
                tokio::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(lp)
                    .await
                    .ok()
            } else {
                None
            };

            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    emit(
                        &tx_clone,
                        SetupProgressEvent::StepProgress {
                            step: "install_deps".into(),
                            progress: -1.0,
                            detail: format!("[{}] {}", srv_name, line),
                        },
                    );
                    // Append to log file
                    if let Some(ref mut writer) = log_writer {
                        use tokio::io::AsyncWriteExt;
                        let ts = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S");
                        let _ = writer
                            .write_all(format!("[{ts}] [{srv_name}] {line}\n").as_bytes())
                            .await;
                    }
                }
                last.push(line);
                if last.len() > 5 {
                    last.remove(0);
                }
            }
            last
        })
    });
    // stdout: capture but don't stream (uv puts little on stdout)
    let stdout_handle = child.stdout.take().map(|stdout| {
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(_)) = lines.next_line().await {}
        })
    });

    match tokio::time::timeout(std::time::Duration::from_secs(120), child.wait()).await {
        Ok(Ok(exit)) if exit.success() => {
            if let Some(h) = stderr_stream {
                let _ = h.await;
            }
            if let Some(h) = stdout_handle {
                let _ = h.await;
            }
            Ok(())
        }
        Ok(Ok(_)) => {
            let tail = if let Some(h) = stderr_stream {
                h.await.unwrap_or_default()
            } else {
                vec![]
            };
            Err(format!("uv pip install failed: {}", tail.join(" | ")))
        }
        Ok(Err(e)) => Err(format!("failed to wait for uv: {e}")),
        Err(_) => {
            let _ = child.kill().await;
            Err("timed out (120s)".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uv_download_target() {
        let (target, ext) = uv_download_target();
        assert!(!target.is_empty());
        assert!(ext == "zip" || ext == "tar.gz");
    }

    #[test]
    fn test_setup_complete_file_backward_compat() {
        // Old format with python_version should deserialize without error
        let json = r#"{
            "completed_at": "2026-04-07T00:00:00Z",
            "version": "0.6.3",
            "mcp_servers_version": "0.1.2",
            "python_version": "3.13.3",
            "server_count": 10
        }"#;
        let file: SetupCompleteFile = serde_json::from_str(json).unwrap();
        assert_eq!(file.server_count, 10);
        assert!(file.uv_version.is_none());
    }
}
