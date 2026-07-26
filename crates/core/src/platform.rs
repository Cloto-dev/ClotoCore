//! Platform-specific service management, permissions, and binary update operations.
//! Each platform module exposes the same public interface, selected at compile time via #[cfg].

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

/// Poll until `pid` is gone, up to `timeout`. Returns `true` once the process
/// is no longer alive, `false` if it outlived the wait.
///
/// Shared by every platform: only the liveness probe itself
/// (`is_process_alive`) is OS-specific.
///
/// Used by the uninstall helper's handoff (`DEFENDER_DESIGN.md` §7), which must
/// not touch files the parent still holds open — so the caller treats `false`
/// as "remove nothing". The updater's `execute_swap` predates this helper and
/// keeps its own inline wait; unifying the two would change the update path,
/// which is out of scope here.
#[must_use]
pub fn wait_for_process_exit(pid: u32, timeout: std::time::Duration) -> bool {
    const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

    let deadline = std::time::Instant::now() + timeout;
    loop {
        if !is_process_alive(pid) {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// A child that stays up long enough to be observed, without depending on
    /// anything outside the base OS install.
    fn spawn_sleeper() -> std::process::Child {
        let mut cmd = if cfg!(windows) {
            let mut c = std::process::Command::new("ping");
            c.args(["-n", "30", "127.0.0.1"]);
            c
        } else {
            let mut c = std::process::Command::new("sleep");
            c.arg("30");
            c
        };
        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("the base OS provides this command")
    }

    #[test]
    fn a_running_process_is_not_reported_as_exited() {
        // The whole uninstall handoff rests on this one property: the detached
        // helper starts deleting the moment this returns true, so a live kernel
        // must never look gone. Nothing else in the tree exercises it.
        let mut child = spawn_sleeper();
        let pid = child.id();

        let observed = wait_for_process_exit(pid, Duration::from_millis(200));

        // Reap before asserting, so a failure does not also leak the child.
        let _ = child.kill();
        let _ = child.wait();
        assert!(
            !observed,
            "a process that is still running must not be reported as exited"
        );
    }

    #[test]
    fn a_reaped_process_is_reported_as_exited() {
        let mut child = spawn_sleeper();
        let pid = child.id();
        child.kill().expect("the child is ours to kill");
        child.wait().expect("and ours to reap");

        assert!(
            wait_for_process_exit(pid, Duration::from_secs(5)),
            "an exited process must release the handoff"
        );
    }
}
