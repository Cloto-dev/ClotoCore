//! Spawning a freshly copied binary races the copies other test binaries make.
//!
//! The kernel process tests copy `CARGO_BIN_EXE_clotocore` into a scratch
//! directory so `config::is_dev_layout()` resolves the data dir there instead
//! of `target/debug/data`. `fs::copy` writes through a file descriptor, and
//! cargo runs integration-test binaries in parallel: when another test forks
//! to spawn while that descriptor is still open, the child inherits it, and
//! the OS refuses to exec an image anyone holds open for writing (ETXTBSY,
//! "Text file busy"). The descriptor is close-on-exec, so the window is one
//! fork-to-exec — but it cannot be closed from inside a single test binary,
//! because the race crosses binaries. Serialising within one file does not
//! help. Waiting the window out does.

use std::io::ErrorKind;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

/// How long to keep retrying a spawn the OS refuses as busy. The window being
/// waited out is a single fork-to-exec, so this is far more than it needs.
const BUSY_BUDGET: Duration = Duration::from_secs(10);
const BUSY_INTERVAL: Duration = Duration::from_millis(25);

/// Spawn `cmd`, waiting out the transient "text file busy" refusal above.
///
/// Every other spawn error fails on the first attempt: this retries one
/// specific race, and is not a way to paper over a binary that cannot start.
pub fn spawn_retrying_busy(cmd: &mut Command) -> Child {
    let deadline = Instant::now() + BUSY_BUDGET;
    let mut attempts = 0u32;
    loop {
        attempts += 1;
        match cmd.spawn() {
            Ok(child) => return child,
            Err(e) if e.kind() == ErrorKind::ExecutableFileBusy => {
                assert!(
                    Instant::now() < deadline,
                    "the kernel binary stayed busy for {BUSY_BUDGET:?} across \
                     {attempts} spawn attempts: {e}"
                );
                std::thread::sleep(BUSY_INTERVAL);
            }
            Err(e) => panic!("spawn kernel: {e}"),
        }
    }
}
