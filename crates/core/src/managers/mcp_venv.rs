//! Auto-setup for Python MCP server virtual environment.
//!
//! Checks `[paths].servers` in `mcp.toml` for the servers directory,
//! falling back to `mcp-servers/` in the project root.
//! If missing, creates a venv and installs dependencies from each server's `pyproject.toml`.
//! This is non-fatal — the kernel starts normally even if setup fails.

use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Resolve the project root using McpClientManager's existing detection logic.
fn resolve_project_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let root = super::McpClientManager::detect_project_root(&exe)?;
    Some(root)
}

/// Read `[paths].servers` from `mcp.toml` in the project root.
/// Uses the same `McpConfigFile` typed deserialization as `load_config_file()`
/// to avoid silent failures from generic `toml::Value` parsing.
pub fn resolve_servers_dir_from_config() -> Option<PathBuf> {
    let root = resolve_project_root()?;
    let config_path = root.join("mcp.toml");
    let content = std::fs::read_to_string(&config_path).ok()?;
    let config: super::mcp_protocol::McpConfigFile = toml::from_str(&content).ok()?;
    let raw = config.paths.get("servers")?;
    // Support env var expansion in path values: ${VAR_NAME}
    let resolved = if let Some(var_name) = raw.strip_prefix("${").and_then(|s| s.strip_suffix('}'))
    {
        std::env::var(var_name).ok()?
    } else {
        raw.clone()
    };
    Some(PathBuf::from(resolved))
}

/// Get the path to the shared venv directory.
/// Checks `[paths].servers` in `mcp.toml` first, then falls back to
/// `mcp-servers/.venv` in the project root.
#[must_use]
pub fn resolve_venv_dir() -> Option<PathBuf> {
    // Primary: [paths].servers from mcp.toml
    if let Some(servers_dir) = resolve_servers_dir_from_config() {
        let venv = servers_dir.join(".venv");
        if venv.join("pyvenv.cfg").exists() {
            return Some(venv);
        }
    }
    // Fallback: legacy location
    resolve_project_root().map(|root| root.join("mcp-servers").join(".venv"))
}

/// Get the path to the Python executable inside the venv.
#[must_use]
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
pub(crate) fn find_python() -> Option<String> {
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

/// Install (or re-sync) each MCP server's pyproject.toml dependencies into
/// the shared venv. Uses `pip install --quiet` which is a no-op for already
/// satisfied packages, so this is safe to run on every startup.
pub(crate) async fn install_server_deps(pip_str: &str, mcp_servers_dir: &Path) -> u32 {
    let mut installed = 0u32;
    let Ok(entries) = std::fs::read_dir(mcp_servers_dir) else {
        return 0;
    };
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

            let result = tokio::process::Command::new(pip_str)
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
    installed
}

/// Ensure the MCP Python venv exists and has dependencies installed.
/// Non-fatal: logs warnings on failure but does not prevent kernel startup.
/// Re-syncs dependencies on every startup so late-added servers are picked up.
///
/// If `data_dir` is provided, also syncs marketplace-installed servers from
/// `data_dir/mcp-servers/` so they survive venv recreation.
pub async fn ensure_mcp_venv(data_dir: Option<&Path>) {
    let Some(project_root) = resolve_project_root() else {
        warn!("Could not detect project root — skipping MCP venv setup");
        return;
    };

    // Use [paths].servers from mcp.toml if available, otherwise legacy path
    let (venv_dir, mcp_servers_dir) = if let Some(servers_dir) = resolve_servers_dir_from_config() {
        (servers_dir.join(".venv"), servers_dir)
    } else {
        (
            project_root.join("mcp-servers").join(".venv"),
            project_root.join("mcp-servers"),
        )
    };
    let venv_exists = venv_dir.join("pyvenv.cfg").exists();

    if venv_exists {
        info!("MCP Python venv found at {}", venv_dir.display());
    } else {
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

        // Create venv
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

        // Upgrade pip (only on fresh venv creation)
        let pip = venv_pip(&venv_dir);
        let pip_str = pip.to_string_lossy().to_string();
        let _ = tokio::process::Command::new(&pip_str)
            .args(["install", "--upgrade", "pip", "--quiet"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .status()
            .await;
    }

    // Always sync dependencies — handles late-added servers and new packages.
    let pip = venv_pip(&venv_dir);
    let pip_str = pip.to_string_lossy().to_string();
    let mut installed = install_server_deps(&pip_str, &mcp_servers_dir).await;

    // Also sync marketplace-installed servers from data_dir/mcp-servers/.
    // pip install is idempotent (no-op for satisfied packages), so double-scanning is safe.
    if let Some(data_dir) = data_dir {
        let marketplace_dir = data_dir.join("mcp-servers");
        if marketplace_dir.is_dir() && marketplace_dir != mcp_servers_dir {
            let mp_count = install_server_deps(&pip_str, &marketplace_dir).await;
            if mp_count > 0 {
                info!("Marketplace dep sync: {} server(s) processed", mp_count);
            }
            installed += mp_count;
        }
    }

    info!(
        "MCP venv dep sync complete: {} server(s) processed at {}",
        installed,
        venv_dir.display()
    );
}
