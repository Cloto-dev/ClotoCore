//! Which process, if any, is currently running this installation.
//!
//! The uninstall path needs an answer to that question and the repository had
//! none: nothing in the tree recorded the running kernel's pid. Without it,
//! `clotocore uninstall --execute` will happily remove the data directory of a
//! live installation — on Unix the deletions succeed, the running process goes
//! on writing to unlinked inodes, and on its way out it recreates the receipt
//! and the directory. The command reports a completed uninstall and the
//! installation is still there.
//!
//! The record is advisory, not a mutual-exclusion lock. It answers "is
//! something running?" for a human-driven, once-per-installation operation; it
//! is not load-bearing for concurrency, and a stale file is treated as no
//! holder rather than as a reason to refuse forever.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::debug;

/// Named for what it holds rather than for the mechanism: the file is a record
/// of a process, not an OS lock.
const RUN_LOCK_FILE: &str = "kernel.pid";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RunLock {
    pid: u32,
    /// Which build wrote it — useful in a doctor report, and it makes the file
    /// self-describing for anyone who finds one left behind.
    app_version: String,
    started_at: String,
}

#[must_use]
pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(RUN_LOCK_FILE)
}

/// Record this process as the one running the installation in `data_dir`.
///
/// Best-effort: a kernel that cannot write its own pid file must still boot.
/// The consequence of a missing record is a `--execute` that is not refused,
/// which is the behaviour that shipped before this existed.
pub fn acquire(data_dir: &Path) {
    let lock = RunLock {
        pid: std::process::id(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        started_at: chrono::Utc::now().to_rfc3339(),
    };
    let target = path(data_dir);
    let write = serde_json::to_vec_pretty(&lock)
        .map_err(std::io::Error::other)
        .and_then(|body| std::fs::write(&target, body));
    match write {
        Ok(()) => debug!("run lock written: {}", target.display()),
        Err(e) => debug!("run lock not written ({}): {e}", target.display()),
    }
}

/// Drop the record, but only if it is still ours.
///
/// A second kernel that started after us owns the file now; deleting it would
/// tell the uninstall path that nothing is running while something is.
pub fn release(data_dir: &Path) {
    let target = path(data_dir);
    match read(&target) {
        Some(lock) if lock.pid == std::process::id() => {
            let _ = std::fs::remove_file(&target);
        }
        _ => {}
    }
}

/// The pid of a *live* kernel other than this process, if there is one.
///
/// `None` covers all three benign cases — no file, an unreadable or malformed
/// file, and a pid that is no longer alive — because each of them means the
/// same thing to the caller: nothing is holding the installation open. A pid
/// that has been reused by an unrelated process is reported as a holder; that
/// is the safe direction, since the result is a refusal to delete.
#[must_use]
pub fn live_holder(data_dir: &Path) -> Option<u32> {
    let lock = read(&path(data_dir))?;
    if lock.pid == std::process::id() {
        return None;
    }
    crate::platform::is_process_alive(lock.pid).then_some(lock.pid)
}

fn read(target: &Path) -> Option<RunLock> {
    let text = std::fs::read_to_string(target).ok()?;
    serde_json::from_str(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn our_own_record_is_not_a_holder() {
        // The kernel writes this file and then serves the uninstall request
        // that reads it. If it counted itself, the app could never uninstall
        // itself — which is the entire flow of §7.
        let dir = tempfile::tempdir().unwrap();
        acquire(dir.path());

        assert!(path(dir.path()).is_file(), "the record must be written");
        assert_eq!(
            live_holder(dir.path()),
            None,
            "a process must not detect itself as another running kernel"
        );
    }

    #[test]
    fn a_dead_pid_does_not_hold_the_installation() {
        // The common real case: a machine that lost power mid-run leaves the
        // file behind. Treating that as "still running" would make the
        // uninstall permanently unavailable with no way to clear it.
        let dir = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new(if cfg!(windows) { "ping" } else { "sleep" })
            .args(if cfg!(windows) {
                ["-n", "30", "127.0.0.1"].as_slice()
            } else {
                ["30"].as_slice()
            })
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("the base OS provides this command");
        let pid = child.id();
        child.kill().expect("the child is ours to kill");
        child.wait().expect("and ours to reap");

        write_lock(dir.path(), pid);
        assert_eq!(live_holder(dir.path()), None, "a dead pid holds nothing");
    }

    #[test]
    fn a_live_foreign_pid_holds_the_installation() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new(if cfg!(windows) { "ping" } else { "sleep" })
            .args(if cfg!(windows) {
                ["-n", "30", "127.0.0.1"].as_slice()
            } else {
                ["30"].as_slice()
            })
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("the base OS provides this command");
        let pid = child.id();

        write_lock(dir.path(), pid);
        let observed = live_holder(dir.path());

        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(
            observed,
            Some(pid),
            "a running process must be reported, or the uninstall deletes a live install"
        );
    }

    #[test]
    fn junk_and_absence_read_the_same_way() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(live_holder(dir.path()), None, "no file, no holder");

        std::fs::write(path(dir.path()), b"not json at all").unwrap();
        assert_eq!(
            live_holder(dir.path()),
            None,
            "an unreadable record must not block the uninstall forever"
        );
    }

    #[test]
    fn release_leaves_a_foreign_record_alone() {
        // Restart races: a newer kernel owns the file by the time an older one
        // finishes shutting down. Removing it there would report the live
        // kernel as gone.
        let dir = tempfile::tempdir().unwrap();
        let foreign = std::process::id() + 1;
        write_lock(dir.path(), foreign);

        release(dir.path());
        assert!(
            path(dir.path()).is_file(),
            "only the process named in the record may remove it"
        );

        acquire(dir.path());
        release(dir.path());
        assert!(!path(dir.path()).exists(), "our own record is cleared");
    }

    fn write_lock(data_dir: &Path, pid: u32) {
        let lock = RunLock {
            pid,
            app_version: "test".to_string(),
            started_at: "2026-07-26T00:00:00Z".to_string(),
        };
        std::fs::write(path(data_dir), serde_json::to_vec(&lock).unwrap()).unwrap();
    }
}
