//! Auto-setup for Python MCP server virtual environment.
//!
//! Server files live in `{data_dir}/mcp-servers/`, with a shared venv at
//! `{data_dir}/mcp-servers/.venv`.  In dev mode, falls back to
//! `mcp-servers/.venv` in the project root.
//! If missing, creates a venv and installs dependencies from each server's `pyproject.toml`.
//! This is non-fatal — the kernel starts normally even if setup fails.
//!
//! Uses `uv` (Rust-based Python package manager) for all venv and dependency
//! operations. Python 3.13 is the pinned target version, provisioned automatically
//! by uv if not present on the system.

use std::path::{Path, PathBuf};
use tracing::{info, warn};

use crate::CHILD_PROCESS_TIMEOUT_SECS;

/// Target Python version for venv creation.
pub const TARGET_PYTHON: &str = "3.13";

/// Resolve the project root using McpClientManager's existing detection logic.
fn resolve_project_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let root = super::McpClientManager::detect_project_root(&exe)?;
    Some(root)
}

/// Resolve the MCP servers directory.
/// Primary: `{data_dir}/mcp-servers` (production).
/// Fallback: `CLOTO_MCP_SERVERS` env var, then `../cloto-mcp-servers/servers` relative to project root (dev).
#[must_use]
pub fn resolve_servers_dir_from_config() -> Option<PathBuf> {
    // 1. Production: {data_dir}/mcp-servers
    let data_servers = crate::config::exe_dir().join("data").join("mcp-servers");
    if data_servers.is_dir() {
        return Some(data_servers);
    }
    // 2. CLOTO_MCP_SERVERS env var
    if let Ok(val) = std::env::var("CLOTO_MCP_SERVERS") {
        let p = PathBuf::from(&val);
        if p.is_dir() {
            return Some(p);
        }
    }
    // 3. Dev: ../cloto-mcp-servers/servers relative to project root
    if let Some(root) = resolve_project_root() {
        let dev_path = root.join("..").join("cloto-mcp-servers").join("servers");
        if dev_path.is_dir() {
            return Some(dev_path);
        }
    }
    None
}

/// Get the path to the shared venv directory.
/// Primary: `{data_dir}/mcp-servers/.venv` (production).
/// Fallback: dev project root, then `CLOTO_MCP_SERVERS`.
#[must_use]
pub fn resolve_venv_dir() -> Option<PathBuf> {
    // Primary: production data dir
    let data_venv = crate::config::exe_dir()
        .join("data")
        .join("mcp-servers")
        .join(".venv");
    if data_venv.join("pyvenv.cfg").exists() {
        return Some(data_venv);
    }
    // Fallback: resolved servers dir (includes dev and env var paths)
    if let Some(servers_dir) = resolve_servers_dir_from_config() {
        let venv = servers_dir.join(".venv");
        if venv.join("pyvenv.cfg").exists() {
            return Some(venv);
        }
    }
    // Fallback: dev project root
    if let Some(root) = resolve_project_root() {
        let venv = root.join("mcp-servers").join(".venv");
        if venv.join("pyvenv.cfg").exists() {
            return Some(venv);
        }
    }
    None
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

/// Get the path to the uv binary in the data directory.
#[must_use]
pub fn uv_bin(data_dir: &Path) -> PathBuf {
    let bin_dir = data_dir.join("bin");
    if cfg!(windows) {
        bin_dir.join("uv.exe")
    } else {
        bin_dir.join("uv")
    }
}

/// Get the Python executable path inside a given venv directory.
#[must_use]
pub fn venv_python(venv_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        venv_dir.join("Scripts").join("python.exe")
    } else {
        venv_dir.join("bin").join("python")
    }
}

/// Extract the Python version from a venv's `pyvenv.cfg` (e.g., "3.13.3").
///
/// Handles both pip/native venv (`version = 3.13.3`) and uv (`version_info = 3.13.3`).
fn read_venv_python_version(venv_dir: &Path) -> Option<String> {
    let cfg_path = venv_dir.join("pyvenv.cfg");
    let content = std::fs::read_to_string(cfg_path).ok()?;
    for line in content.lines() {
        let line = line.trim();
        // Match "version_info = ..." (uv) first, then "version = ..." (pip).
        // Must not match "version_info" when looking for "version".
        if let Some(rest) = line.strip_prefix("version_info") {
            let version = rest.trim_start_matches([' ', '=']);
            let version = version.trim();
            if !version.is_empty() {
                return Some(version.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("version") {
            let rest = rest.trim_start();
            if let Some(version) = rest.strip_prefix('=') {
                let version = version.trim();
                if !version.is_empty() {
                    return Some(version.to_string());
                }
            }
        }
    }
    None
}

/// Check if an existing venv's Python version mismatches the target (3.13).
/// Returns `true` if the venv is stale and should be recreated.
/// Returns `false` if the venv is OK, or if the version cannot be determined.
pub(crate) fn is_venv_stale(venv_dir: &Path) -> bool {
    let Some(venv_version) = read_venv_python_version(venv_dir) else {
        return false; // Can't read venv version — assume OK
    };
    // With uv-managed Python, stale means anything other than TARGET_PYTHON (3.13.x)
    if !venv_version.starts_with(TARGET_PYTHON) {
        tracing::info!(
            venv_python = %venv_version,
            target = TARGET_PYTHON,
            "Venv Python version mismatch detected"
        );
        return true;
    }
    false
}

/// Install (or re-sync) each MCP server's pyproject.toml dependencies into
/// the shared venv. Uses a single `uv pip install` invocation with all server
/// paths to avoid lock contention from parallel processes.
pub(crate) async fn install_server_deps(
    uv_str: &str,
    venv_python_str: &str,
    mcp_servers_dir: &Path,
) -> u32 {
    let Ok(entries) = std::fs::read_dir(mcp_servers_dir) else {
        return 0;
    };

    // Collect valid server directories (with pyproject.toml)
    let mut server_paths: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        // Skip .venv, common, tests, and non-Python servers (Rust: discord, avatar)
        if name.starts_with('.')
            || name == "common"
            || name == "tests"
            || !path.join("pyproject.toml").exists()
        {
            continue;
        }
        // Skip Rust servers (they have Cargo.toml, not pip-installable)
        if path.join("Cargo.toml").exists() {
            continue;
        }
        server_paths.push(path.to_string_lossy().to_string());
    }

    if server_paths.is_empty() {
        return 0;
    }

    let count = server_paths.len() as u32;
    info!("  Installing MCP deps for {} server(s) via uv", count);

    // Single uv invocation with all server paths
    let mut args = vec![
        "pip".to_string(),
        "install".to_string(),
        "--no-progress".to_string(),
        "--python".to_string(),
        venv_python_str.to_string(),
    ];
    args.extend(server_paths);

    let result = tokio::process::Command::new(uv_str)
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await;

    match result {
        Ok(output) if output.status.success() => count,
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let last_line = stderr.lines().last().unwrap_or("unknown error");
            warn!(
                "  uv pip install failed (exit code {:?}): {}",
                output.status.code(),
                last_line
            );
            0
        }
        Err(e) => {
            warn!("  Failed to run uv: {}", e);
            0
        }
    }
}

/// Ensure the MCP Python venv exists and has dependencies installed.
/// Non-fatal: logs warnings on failure but does not prevent kernel startup.
/// Re-syncs dependencies on every startup so late-added servers are picked up.
///
/// Also syncs marketplace-installed servers from `data_dir/mcp-servers/`
/// so they survive venv recreation.
pub async fn ensure_mcp_venv(data_dir: &Path) {
    let Some(project_root) = resolve_project_root() else {
        warn!("Could not detect project root — skipping MCP venv setup");
        return;
    };

    // Check uv binary availability
    let uv = uv_bin(data_dir);
    if !uv.exists() {
        warn!(
            "uv binary not found at {} — skipping MCP venv setup. \
             Run initial setup first.",
            uv.display()
        );
        return;
    }
    let uv_str = uv.to_string_lossy().to_string();

    // Use [paths].servers from mcp.toml if available, otherwise legacy path
    let (venv_dir, mcp_servers_dir) = if let Some(servers_dir) = resolve_servers_dir_from_config() {
        (servers_dir.join(".venv"), servers_dir)
    } else {
        (
            project_root.join("mcp-servers").join(".venv"),
            project_root.join("mcp-servers"),
        )
    };
    let mut venv_exists = venv_dir.join("pyvenv.cfg").exists();

    // Detect stale venv (Python version mismatch with target 3.13) and recreate
    if venv_exists && is_venv_stale(&venv_dir) {
        warn!(
            "Python version mismatch (expected {}) — recreating venv at {}",
            TARGET_PYTHON,
            venv_dir.display()
        );
        let _ = std::fs::remove_dir_all(&venv_dir);
        venv_exists = false;
    }

    if venv_exists {
        info!("MCP Python venv found at {}", venv_dir.display());
    } else {
        info!("MCP Python venv not found — creating via uv...");

        let venv_path_str = venv_dir.to_string_lossy().to_string();
        let create_result = tokio::time::timeout(
            std::time::Duration::from_secs(CHILD_PROCESS_TIMEOUT_SECS),
            tokio::process::Command::new(&uv_str)
                .args(["venv", "--python", TARGET_PYTHON, &venv_path_str])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .status(),
        )
        .await;

        match create_result {
            Ok(Ok(status)) if status.success() => {
                info!(
                    "Created Python {} venv at {}",
                    TARGET_PYTHON,
                    venv_dir.display()
                );
            }
            Ok(Ok(status)) => {
                warn!(
                    "Failed to create venv via uv (exit code: {:?})",
                    status.code()
                );
                return;
            }
            Ok(Err(e)) => {
                warn!("Failed to run uv for venv creation: {}", e);
                return;
            }
            Err(_) => {
                warn!("uv venv creation timed out (120s)");
                return;
            }
        }
    }

    // Always sync dependencies — handles late-added servers and new packages.
    let venv_py = venv_python(&venv_dir);
    let venv_py_str = venv_py.to_string_lossy().to_string();
    let mut installed = install_server_deps(&uv_str, &venv_py_str, &mcp_servers_dir).await;

    // Also sync marketplace-installed servers from data_dir/mcp-servers/.
    // uv install is idempotent (no-op for satisfied packages), so double-scanning is safe.
    let marketplace_dir = data_dir.join("mcp-servers");
    if marketplace_dir.is_dir() && marketplace_dir != mcp_servers_dir {
        let mp_count = install_server_deps(&uv_str, &venv_py_str, &marketplace_dir).await;
        if mp_count > 0 {
            info!("Marketplace dep sync: {} server(s) processed", mp_count);
        }
        installed += mp_count;
    }

    info!(
        "MCP venv dep sync complete: {} server(s) processed at {}",
        installed,
        venv_dir.display()
    );
}
