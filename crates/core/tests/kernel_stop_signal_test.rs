//! The kernel binary must stop cleanly when its supervisor asks it to.
//!
//! A service manager (systemd, launchd, a container runtime) stops a daemon
//! with SIGTERM. It holds no API key, so `/api/system/shutdown` is not an
//! option for it. Before `run_kernel` listened for the signal, SIGTERM killed
//! the process outright (exit 143) and every MCP child it had spawned was
//! orphaned. This test drives the real binary and asserts the graceful path.
#![cfg(unix)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

/// Minimal HTTP probe: true once `/api/system/health` answers 200.
fn health_ok(port: u16) -> bool {
    let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    if s.write_all(b"GET /api/system/health HTTP/1.0\r\nHost: localhost\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut buf = String::new();
    let _ = s.read_to_string(&mut buf);
    buf.starts_with("HTTP/1.1 200") || buf.starts_with("HTTP/1.0 200")
}

#[test]
fn sigterm_shuts_the_kernel_down_gracefully() {
    // Run the binary from a scratch directory that carries a dummy workspace
    // manifest: `config::is_dev_layout()` then resolves `data_dir` next to the
    // executable, so the database, sandbox and logs all stay inside the
    // temp dir instead of touching `target/debug/data`.
    let root = tempfile::tempdir().expect("tempdir");
    std::fs::write(root.path().join("Cargo.toml"), "[workspace]\n").unwrap();
    let bin_dir = root.path().join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let exe = bin_dir.join("clotocore");
    std::fs::copy(env!("CARGO_BIN_EXE_clotocore"), &exe).expect("copy kernel binary");

    let port = free_port();
    let proxy_port = free_port();
    let sandbox = root.path().join("sandbox");
    std::fs::create_dir_all(&sandbox).unwrap();
    let db_url = format!("sqlite:{}", root.path().join("kernel.sqlite3").display());

    let mut child = Command::new(&exe)
        .current_dir(root.path())
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", std::env::var("HOME").unwrap_or_default())
        .env(
            "CLOTO_API_KEY",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .env("PORT", port.to_string())
        .env("BIND_ADDRESS", "127.0.0.1")
        .env("DATABASE_URL", db_url)
        .env("CLOTO_SANDBOX_DIR", &sandbox)
        .env("CLOTO_LLM_PROXY_PORT", proxy_port.to_string())
        .env("RUST_LOG", "info")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn kernel");

    // Drain stdout/stderr on threads so the child never blocks on a full pipe.
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let out_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let err_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });

    let boot = Instant::now();
    while !health_ok(port) {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("kernel exited before becoming healthy: {status}");
        }
        assert!(
            boot.elapsed() < Duration::from_secs(90),
            "kernel did not answer /api/system/health within 90s"
        );
        std::thread::sleep(Duration::from_millis(250));
    }

    // SAFETY: `child.id()` is the pid of a process this test spawned and still
    // owns; sending it SIGTERM is exactly the supervisor behaviour under test.
    let rc = unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
    assert_eq!(rc, 0, "kill(SIGTERM) failed");

    let stop = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if stop.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            panic!("kernel did not exit within 30s of SIGTERM");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let logs = format!("{}\n{}", out_t.join().unwrap(), err_t.join().unwrap());

    assert!(
        status.success(),
        "expected a clean exit after SIGTERM, got {status}\n--- kernel log ---\n{logs}"
    );
    assert!(
        logs.contains("Stop signal received from the OS"),
        "the signal arm did not run\n--- kernel log ---\n{logs}"
    );
    assert!(
        logs.contains("Graceful shutdown signal received"),
        "the HTTP server did not go through graceful shutdown\n--- kernel log ---\n{logs}"
    );
}
