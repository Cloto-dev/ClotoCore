use anyhow::{bail, Context};
use std::path::Path;
use std::process::Command;
use tracing::info;

const SERVICE_NAME: &str = "cloto";
const SERVICE_FILE: &str = "/etc/systemd/system/cloto.service";

/// Generate systemd service unit file content.
///
/// Written for an unattended daemon: it waits for the network to be up (the
/// kernel reaches upstream providers and the marketplace), stops with SIGTERM
/// and enough time to drain MCP children (`run_kernel` handles the signal;
/// the drain is bounded at ~10s per shutdown), logs to the journal, gives the
/// children an explicit PATH (`python3`, `node`, `git` are looked up there),
/// and keeps the system tree read-only — everything the kernel writes lives
/// under the prefix or the service user's data directory.
fn service_unit(prefix: &Path, user: &str) -> String {
    let exec_start = prefix.join("clotocore");
    format!(
        r"[Unit]
Description=ClotoCore
Wants=network-online.target
After=network-online.target

[Service]
Type=simple
User={user}
WorkingDirectory={prefix}
ExecStart={exec_start}
Restart=on-failure
RestartSec=5
EnvironmentFile={prefix}/.env
Environment=PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
KillSignal=SIGTERM
KillMode=mixed
TimeoutStopSec=30
StandardOutput=journal
StandardError=journal
LimitNOFILE=65536
NoNewPrivileges=true
ProtectSystem=full
PrivateTmp=true

[Install]
WantedBy=multi-user.target
",
        user = user,
        prefix = prefix.display(),
        exec_start = exec_start.display(),
    )
}

/// Register Cloto as a systemd service.
///
/// The unit runs as `user`, or as root when none is given: the installer
/// itself runs as root (it writes under `/opt` and `/etc`), and a different
/// service user must also own the prefix — `installer::install` chowns it
/// when `--user` is passed.
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

/// Remove Cloto systemd service
/// Deregister the systemd service.
///
/// `Ok(true)` = a registration was removed, `Ok(false)` = there was none,
/// `Err` = the removal failed. The purge executor reports these as `removed` /
/// `absent` / `failed`, so collapsing "nothing to remove" into success would
/// make it claim it deleted something that was never there.
pub fn uninstall_service() -> anyhow::Result<bool> {
    // Stop if running (ignore errors)
    let _ = run_systemctl(&["stop", SERVICE_NAME]);
    let _ = run_systemctl(&["disable", SERVICE_NAME]);

    if !Path::new(SERVICE_FILE).exists() {
        info!("ℹ️  Service file not found, nothing to remove");
        return Ok(false);
    }
    std::fs::remove_file(SERVICE_FILE).context("Failed to remove service file")?;
    run_systemctl(&["daemon-reload"])?;
    info!("✅ Service removed: {}", SERVICE_NAME);
    Ok(true)
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
        bail!(
            "systemctl {} failed with exit code {:?}",
            args.join(" "),
            status.code()
        );
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
pub fn swap_running_binary(
    new_path: &Path,
    current_path: &Path,
    old_path: &Path,
) -> anyhow::Result<()> {
    // Remove previous backup if exists (ignore NotFound)
    match std::fs::remove_file(old_path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(anyhow::anyhow!(
                "Failed to remove old backup {}: {}",
                old_path.display(),
                e
            ))
        }
    }

    // current → old (backup)
    std::fs::rename(current_path, old_path).with_context(|| {
        format!(
            "Failed to backup current binary: {}",
            current_path.display()
        )
    })?;

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

/// Check if a process is alive by PID (Unix: the signal-0 existence probe).
///
/// `EPERM` counts as alive: the process exists, it simply belongs to another
/// user. Reading that as "gone" would let the uninstall helper start deleting
/// files while the kernel it is waiting for is still running.
/// A pid that does not fit `pid_t` cannot be probed, so it is reported alive:
/// the safe reading of "unintelligible" is "still running", because the caller
/// deletes files once this returns false.
#[must_use]
pub fn is_process_alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return true;
    };
    // SAFETY: `kill` with signal 0 performs no action; it only reports whether
    // the pid can be signalled.
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Registry keys are a Windows concept; the signature exists so the purge
/// executor is written once for every platform.
pub fn delete_registry_key(key_path: &str) -> anyhow::Result<bool> {
    bail!("registry keys exist only on Windows (asked for {key_path})")
}

/// Execute binary swap (direct rename on Unix — called inline, no subprocess needed)
pub fn execute_swap(target: std::path::PathBuf, _pid: u32) -> anyhow::Result<()> {
    // On Unix, swap-exe is not used (rename works on running files).
    // This exists for CLI completeness but should not normally be called on Linux.
    info!("swap-exe is a no-op on Unix (rename works on running files)");
    let _ = target;
    Ok(())
}

/// Start `exe` and return without waiting for it.
///
/// Used by the uninstall handoff (§7). The child is put in its own process
/// group: it has to outlive this process, and a group-wide signal aimed at the
/// exiting kernel — a terminal closing, a service manager stopping the unit —
/// would otherwise take the helper down with it, leaving an installation
/// half-removed.
pub fn spawn_detached(exe: &Path, args: &[String]) -> anyhow::Result<()> {
    use std::os::unix::process::CommandExt as _;

    Command::new(exe)
        .args(args)
        .process_group(0)
        .stdin(std::process::Stdio::null())
        .spawn()
        .with_context(|| format!("Failed to spawn {}", exe.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_unit_is_written_for_an_unattended_daemon() {
        let unit = service_unit(Path::new("/opt/cloto"), "cloto");
        for line in [
            "User=cloto",
            "WorkingDirectory=/opt/cloto",
            "ExecStart=/opt/cloto/clotocore",
            "EnvironmentFile=/opt/cloto/.env",
            "After=network-online.target",
            "Wants=network-online.target",
            "KillSignal=SIGTERM",
            "TimeoutStopSec=30",
            "StandardOutput=journal",
            "StandardError=journal",
            "Environment=PATH=",
            "ProtectSystem=full",
            "Restart=on-failure",
        ] {
            assert!(unit.contains(line), "unit lacks `{line}`:\n{unit}");
        }
    }
}
