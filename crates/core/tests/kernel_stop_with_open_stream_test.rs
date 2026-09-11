//! A client watching an event stream must not be able to stop the kernel from
//! exiting.
//!
//! `kernel_stop_signal_test.rs` already drives SIGTERM against a kernel nobody
//! is talking to, and that kernel exits in about a tenth of a second. It stayed
//! green through the whole period this test was written to close: measured on
//! the deployed kernel, 8 consecutive stops ended in `SIGKILL` 30s after the
//! signal, because a browser had an SSE response open and axum's graceful
//! shutdown waits for every response body to finish. The population that test
//! measures — connections that do not exist — excluded the failure entirely.
//!
//! Measured while diagnosing it (same harness, one arm each): no connection
//! 0.11s, an idle keep-alive socket 0.11s, a half-sent request body 0.10s, an
//! SSE response in flight **never**. hyper closes the first three itself; only
//! a body the kernel's own code refuses to end can hold the process open.
#![cfg(unix)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[path = "common/kernel_spawn.rs"]
mod kernel_spawn;
use kernel_spawn::spawn_retrying_busy;

/// The stream must end because the handler ended it, not because the exit
/// path's deadline (`SERVER_DRAIN_DEADLINE`, 10s) gave up on it. Sitting
/// between the two is what makes this assertion able to tell them apart.
const COOPERATIVE_EXIT_BUDGET: Duration = Duration::from_secs(5);

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

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
fn a_watched_event_stream_does_not_hold_the_kernel_open_at_sigterm() {
    // Scratch directory with a workspace manifest: `config::is_dev_layout()`
    // then keeps the database, sandbox and logs next to this executable
    // instead of touching `target/debug/data`.
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

    let mut cmd = Command::new(&exe);
    cmd.current_dir(root.path())
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
        .stderr(Stdio::piped());
    let mut child = spawn_retrying_busy(&mut cmd);

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

    // Every streaming route the kernel serves, open at once. One of them ending
    // itself is not the property under test: any single stream that keeps its
    // body open is enough to hold the whole process past the signal, so a test
    // covering one route would go green while the other two were broken.
    let mut streams: Vec<(&str, TcpStream)> = Vec::new();
    for (path, auth) in [
        ("/api/events", true),
        ("/api/marketplace/progress", false),
        ("/api/setup/progress", false),
    ] {
        let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        s.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
        let auth_header = if auth {
            "x-api-key: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\r\n"
        } else {
            ""
        };
        s.write_all(
            format!(
                "GET {path} HTTP/1.1\r\nHost: localhost\r\nAccept: text/event-stream\r\n{auth_header}\r\n"
            )
            .as_bytes(),
        )
        .unwrap();
        // Read the headers so the response is provably in flight rather than
        // merely requested — an unanswered request does not hold the server
        // open (measured: 0.10s), so a test that skipped this would pass
        // whether or not the streams were fixed.
        let mut head = [0u8; 4096];
        let n = s.read(&mut head).unwrap_or(0);
        let head = String::from_utf8_lossy(&head[..n]).to_string();
        assert!(
            head.contains(" 200 ") && head.contains("text/event-stream"),
            "{path} did not start streaming, so this test would not be measuring it: {head}"
        );
        streams.push((path, s));
    }

    // SAFETY: the pid belongs to a process this test spawned and still owns.
    let rc = unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
    assert_eq!(rc, 0, "kill(SIGTERM) failed");

    let stop = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(
            stop.elapsed() < Duration::from_secs(30),
            "the kernel never exited while an event stream was open — \
             this is the defect itself, not a slow machine"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    let waited = stop.elapsed();
    let logs = format!("{}\n{}", out_t.join().unwrap(), err_t.join().unwrap());

    assert!(
        status.success(),
        "expected a clean exit after SIGTERM with a stream open, got {status}\
         \n--- kernel log ---\n{logs}"
    );
    assert!(
        waited < COOPERATIVE_EXIT_BUDGET,
        "the kernel took {waited:?} to exit: the stream did not end itself, and only the \
         exit path's deadline stopped the wait\n--- kernel log ---\n{logs}"
    );
    assert!(
        !logs.contains("did not finish draining"),
        "the exit path had to abandon the HTTP server, so the stream never ended\
         \n--- kernel log ---\n{logs}"
    );

    // The client's side of the same fact, per route: each body ended, rather
    // than the socket being left open until the process died under it. Reading
    // them one at a time is what localizes a regression to the handler that
    // stopped ending its stream.
    for (path, mut s) in streams {
        let mut rest = Vec::new();
        s.read_to_end(&mut rest).unwrap_or_else(|e| {
            panic!("{path} never ended its response body ({e})\n--- kernel log ---\n{logs}")
        });
    }
}
