//! Auto-setup for Python MCP server virtual environment.
//!
//! On first kernel startup, detects if `mcp-servers/.venv` exists.
//! If missing, creates it and installs dependencies from each server's `pyproject.toml`.
//! This is non-fatal — the kernel starts normally even if setup fails.

use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Resolve the project root using McpClientManager's existing detection logic.
fn resolve_project_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let root = super::McpClientManager::detect_project_root(&exe)?;
    Some(root)
}

/// Get the path to the shared venv directory.
pub fn resolve_venv_dir() -> Option<PathBuf> {
    resolve_project_root().map(|root| root.join("mcp-servers").join(".venv"))
}

/// Get the path to the Python executable inside the venv.
pub fn resolve_venv_python() -> Option<PathBuf> {
    let venv_dir = resolve_venv_dir()?;

    // Windows: Scripts/python.exe, Unix: bin/python
    let python = if cfg!(windows) {
        venv_dir.join("Scripts").join("python.exe")
    } else {
        venv_dir.join("bin").join("python")
    };

    if python.exists() {
        Some(python)
    } else {
        None
    }
}

/// Find a system Python command (python3 or python).
fn find_python() -> Option<String> {
    for cmd in &["python3", "python"] {
        let result = std::process::Command::new(cmd)
            .arg("--version")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        if let Ok(mut child) = result {
            if let Ok(status) = child.wait() {
                if status.success() {
                    return Some((*cmd).to_string());
                }
            }
        }
    }
    None
}

/// Get the pip executable path inside the venv.
fn venv_pip(venv_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        venv_dir.join("Scripts").join("pip.exe")
    } else {
        venv_dir.join("bin").join("pip")
    }
}

/// Ensure the MCP Python venv exists and has dependencies installed.
/// Non-fatal: logs warnings on failure but does not prevent kernel startup.
pub async fn ensure_mcp_venv() {
    let Some(project_root) = resolve_project_root() else {
        warn!("Could not detect project root — skipping MCP venv setup");
        return;
    };

    let venv_dir = project_root.join("mcp-servers").join(".venv");
    let mcp_servers_dir = project_root.join("mcp-servers");

    // Check if venv already exists (pyvenv.cfg is the reliable indicator)
    if venv_dir.join("pyvenv.cfg").exists() {
        info!("MCP Python venv found at {}", venv_dir.display());
        return;
    }

    info!("MCP Python venv not found — setting up automatically...");

    // Find system Python
    let Some(python) = find_python() else {
        warn!(
            "Python 3.10+ not found in PATH. MCP servers requiring Python will not start. \
             Install Python and restart, or run: bash scripts/setup-mcp-deps.sh"
        );
        return;
    };

    info!("Using system Python: {}", python);

    // Step 1: Create venv
    let venv_path_str = venv_dir.to_string_lossy().to_string();
    let create_result = tokio::process::Command::new(&python)
        .args(["-m", "venv", &venv_path_str])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .status()
        .await;

    match create_result {
        Ok(status) if status.success() => {
            info!("Created Python venv at {}", venv_dir.display());
        }
        Ok(status) => {
            warn!(
                "Failed to create Python venv (exit code: {:?}). \
                 Run manually: bash scripts/setup-mcp-deps.sh",
                status.code()
            );
            return;
        }
        Err(e) => {
            warn!("Failed to run Python for venv creation: {}", e);
            return;
        }
    }

    // Step 2: Upgrade pip
    let pip = venv_pip(&venv_dir);
    let pip_str = pip.to_string_lossy().to_string();
    let _ = tokio::process::Command::new(&pip_str)
        .args(["install", "--upgrade", "pip", "--quiet"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .status()
        .await;

    // Step 3: Install each server's dependencies
    let mut installed = 0u32;
    if let Ok(entries) = std::fs::read_dir(&mcp_servers_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let name = entry.file_name().to_string_lossy().to_string();

            // Skip .venv and common (not a standalone server)
            if name.starts_with('.') || name == "common" {
                continue;
            }

            if path.join("pyproject.toml").exists() {
                let server_path = path.to_string_lossy().to_string();
                info!("  Installing MCP deps: {}", name);

                let result = tokio::process::Command::new(&pip_str)
                    .args(["install", &server_path, "--quiet"])
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .output()
                    .await;

                match result {
                    Ok(output) if output.status.success() => {
                        installed += 1;
                    }
                    Ok(output) => {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        warn!(
                            "  Failed to install {} (exit code {:?}): {}",
                            name,
                            output.status.code(),
                            stderr.lines().last().unwrap_or("unknown error")
                        );
                    }
                    Err(e) => {
                        warn!("  Failed to run pip for {}: {}", name, e);
                    }
                }
            }
        }
    }

    info!(
        "MCP venv setup complete: {} server(s) installed at {}",
        installed,
        venv_dir.display()
    );
}
