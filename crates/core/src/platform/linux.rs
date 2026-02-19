use anyhow::{Context, bail};
use std::path::Path;
use std::process::Command;
use tracing::info;

const SERVICE_NAME: &str = "exiv";
const SERVICE_FILE: &str = "/etc/systemd/system/exiv.service";

/// Generate systemd service unit file content
fn service_unit(prefix: &Path, user: &str) -> String {
    let exec_start = prefix.join("exiv_system");
    format!(
        r"[Unit]
Description=Exiv System
After=network.target

[Service]
Type=simple
User={user}
WorkingDirectory={prefix}
ExecStart={exec_start}
Restart=on-failure
RestartSec=5
EnvironmentFile={prefix}/.env

[Install]
WantedBy=multi-user.target
",
        user = user,
        prefix = prefix.display(),
        exec_start = exec_start.display(),
    )
}

/// Register Exiv as a systemd service
pub fn install_service(prefix: &Path, user: Option<&str>) -> anyhow::Result<()> {
    let user = user.unwrap_or("root");

    let unit = service_unit(prefix, user);
    info!("📝 Writing systemd service to {}", SERVICE_FILE);

    // Write service file (requires root)
    std::fs::write(SERVICE_FILE, &unit)
        .context("Failed to write systemd service file (are you running as root?)")?;

    // Reload systemd and enable
    run_systemctl(&["daemon-reload"])?;
    run_systemctl(&["enable", SERVICE_NAME])?;

    info!("✅ Service registered: {}", SERVICE_NAME);
    info!("   Start with: sudo systemctl start {}", SERVICE_NAME);
    info!("   Status:     sudo systemctl status {}", SERVICE_NAME);
    info!("   Logs:       journalctl -u {} -f", SERVICE_NAME);
    Ok(())
}

/// Remove Exiv systemd service
pub fn uninstall_service() -> anyhow::Result<()> {
    // Stop if running (ignore errors)
    let _ = run_systemctl(&["stop", SERVICE_NAME]);
    let _ = run_systemctl(&["disable", SERVICE_NAME]);

    if Path::new(SERVICE_FILE).exists() {
        std::fs::remove_file(SERVICE_FILE)
            .context("Failed to remove service file")?;
        run_systemctl(&["daemon-reload"])?;
        info!("✅ Service removed: {}", SERVICE_NAME);
    } else {
        info!("ℹ️  Service file not found, nothing to remove");
    }
    Ok(())
}

pub fn start_service() -> anyhow::Result<()> {
    run_systemctl(&["start", SERVICE_NAME])
}

pub fn stop_service() -> anyhow::Result<()> {
    run_systemctl(&["stop", SERVICE_NAME])
}

pub fn service_status() -> anyhow::Result<String> {
    let output = Command::new("systemctl")
        .args(["status", SERVICE_NAME])
        .output()
        .context("Failed to run systemctl")?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn run_systemctl(args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new("systemctl")
        .args(args)
        .status()
        .with_context(|| format!("Failed to run: systemctl {}", args.join(" ")))?;
    if !status.success() {
        bail!("systemctl {} failed with exit code {:?}", args.join(" "), status.code());
    }
    Ok(())
}

/// Set executable permission on a file (chmod 0o755)
pub fn set_executable_permission(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o755);
    std::fs::set_permissions(path, perms)
        .with_context(|| format!("Failed to set executable permission on {}", path.display()))?;
    Ok(())
}

/// Swap a running binary (Unix: rename is safe even while running)
pub fn swap_running_binary(new_path: &Path, current_path: &Path, old_path: &Path) -> anyhow::Result<()> {
    // Remove previous backup if exists (ignore NotFound)
    match std::fs::remove_file(old_path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(anyhow::anyhow!("Failed to remove old backup {}: {}", old_path.display(), e)),
    }

    // current → old (backup)
    std::fs::rename(current_path, old_path)
        .with_context(|| format!("Failed to backup current binary: {}", current_path.display()))?;

    // new → current (activate)
    std::fs::rename(new_path, current_path).map_err(|e| {
        // Attempt rollback on failure
        match std::fs::rename(old_path, current_path) {
            Ok(()) => anyhow::anyhow!("Failed to install new binary (rolled back): {}", e),
            Err(rb_err) => {
                eprintln!("CRITICAL: Binary install failed and rollback also failed! install_err={}, rollback_err={}", e, rb_err);
                anyhow::anyhow!("Failed to install new binary AND rollback failed: install={}, rollback={}", e, rb_err)
            }
        }
    })?;

    Ok(())
}

/// Execute binary swap (direct rename on Unix — called inline, no subprocess needed)
pub fn execute_swap(target: std::path::PathBuf, _pid: u32) -> anyhow::Result<()> {
    // On Unix, swap-exe is not used (rename works on running files).
    // This exists for CLI completeness but should not normally be called on Linux.
    info!("swap-exe is a no-op on Unix (rename works on running files)");
    let _ = target;
    Ok(())
}
