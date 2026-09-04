//! What the kernel binary does with `BIND_ADDRESS` at startup.
//!
//! Loopback is not a security boundary (a tunnel reaches it regardless), but
//! a non-loopback bind is reachable from other hosts with no helper at all.
//! There the admin API key is the only boundary, so starting without one is
//! refused instead of warned about. The IPv6 case covers the listener that
//! used to be created as an IPv4 socket whatever address it was given.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[path = "common/kernel_spawn.rs"]
mod kernel_spawn;
use kernel_spawn::spawn_retrying_busy;

const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn free_port(host: &str) -> u16 {
    TcpListener::bind((host, 0))
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

/// A scratch install: the binary copied next to a dummy workspace manifest so
/// `config::is_dev_layout()` keeps the data dir inside the temp directory.
struct Scratch {
    root: tempfile::TempDir,
    exe: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::write(root.path().join("Cargo.toml"), "[workspace]\n").unwrap();
        let bin_dir = root.path().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::create_dir_all(root.path().join("sandbox")).unwrap();
        let exe = bin_dir.join("clotocore");
        std::fs::copy(env!("CARGO_BIN_EXE_clotocore"), &exe).expect("copy kernel binary");
        Self { root, exe }
    }

    fn command(&self, bind: &str, port: u16, key: Option<&str>) -> Command {
        let root: &Path = self.root.path();
        let mut cmd = Command::new(&self.exe);
        cmd.current_dir(root).env_clear();
        // A cleared environment must keep what the OS itself needs: on
        // Windows, Winsock fails to initialise without SYSTEMROOT (os error
        // 10106), which surfaces as the kernel failing to bind and exiting.
        for name in [
            "PATH",
            "HOME",
            "SYSTEMROOT",
            "SystemRoot",
            "TEMP",
            "TMP",
            "USERPROFILE",
            "LOCALAPPDATA",
            "APPDATA",
        ] {
            if let Ok(v) = std::env::var(name) {
                cmd.env(name, v);
            }
        }
        cmd.env("PORT", port.to_string())
            .env("BIND_ADDRESS", bind)
            .env(
                "DATABASE_URL",
                format!("sqlite:{}", root.join("kernel.sqlite3").display()),
            )
            .env("CLOTO_SANDBOX_DIR", root.join("sandbox"))
            .env("CLOTO_LLM_PROXY_PORT", free_port("127.0.0.1").to_string())
            .env("RUST_LOG", "info")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(k) = key {
            cmd.env("CLOTO_API_KEY", k);
        }
        cmd
    }
}

fn wait_exit(child: &mut Child, within: Duration) -> Option<std::process::ExitStatus> {
    let start = Instant::now();
    loop {
        if let Some(s) = child.try_wait().unwrap() {
            return Some(s);
        }
        if start.elapsed() > within {
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn collect(child: &mut Child) -> String {
    let mut out = String::new();
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_string(&mut out);
    }
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut out);
    }
    out
}

fn http(host: &str, port: u16, request: &str) -> String {
    let Ok(mut s) = TcpStream::connect((host, port)) else {
        return String::new();
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    if s.write_all(request.as_bytes()).is_err() {
        return String::new();
    }
    let mut buf = String::new();
    let _ = s.read_to_string(&mut buf);
    buf
}

fn health_ok(host: &str, port: u16) -> bool {
    let r = http(
        host,
        port,
        "GET /api/system/health HTTP/1.0\r\nHost: localhost\r\n\r\n",
    );
    r.starts_with("HTTP/1.1 200") || r.starts_with("HTTP/1.0 200")
}

#[test]
fn keyless_kernel_refuses_to_start_on_a_non_loopback_bind() {
    let scratch = Scratch::new();
    let mut cmd = scratch.command("0.0.0.0", free_port("127.0.0.1"), None);
    let mut child = spawn_retrying_busy(&mut cmd);
    let status = wait_exit(&mut child, Duration::from_secs(60)).unwrap_or_else(|| {
        let _ = child.kill();
        panic!("keyless kernel on 0.0.0.0 kept running instead of refusing to start");
    });
    let logs = collect(&mut child);
    assert!(
        !status.success(),
        "expected a startup refusal, got {status}\n--- kernel log ---\n{logs}"
    );
    assert!(
        logs.contains("Refusing to start"),
        "the refusal did not say why\n--- kernel log ---\n{logs}"
    );
}

#[test]
fn keyless_kernel_still_starts_on_loopback() {
    let scratch = Scratch::new();
    let port = free_port("127.0.0.1");
    let mut cmd = scratch.command("127.0.0.1", port, None);
    let mut child = spawn_retrying_busy(&mut cmd);
    let boot = Instant::now();
    while !health_ok("127.0.0.1", port) {
        if let Some(status) = child.try_wait().unwrap() {
            let logs = collect(&mut child);
            panic!("keyless loopback kernel exited: {status}\n--- kernel log ---\n{logs}");
        }
        assert!(
            boot.elapsed() < Duration::from_secs(90),
            "no health within 90s"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn kernel_binds_an_ipv6_loopback_address() {
    if TcpListener::bind("[::1]:0").is_err() {
        eprintln!("skipping: this host has no IPv6 loopback");
        return;
    }
    let scratch = Scratch::new();
    let port = free_port("::1");
    let mut cmd = scratch.command("::1", port, Some(KEY));
    let mut child = spawn_retrying_busy(&mut cmd);
    let boot = Instant::now();
    while !health_ok("::1", port) {
        if let Some(status) = child.try_wait().unwrap() {
            let logs = collect(&mut child);
            panic!("kernel on ::1 exited: {status}\n--- kernel log ---\n{logs}");
        }
        assert!(
            boot.elapsed() < Duration::from_secs(90),
            "no health on ::1 within 90s"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
    // Stop through the authenticated endpoint so the test holds on every OS.
    let r = http(
        "::1",
        port,
        &format!(
            "POST /api/system/shutdown HTTP/1.0\r\nHost: localhost\r\nX-API-Key: {KEY}\r\nContent-Length: 0\r\n\r\n"
        ),
    );
    assert!(
        r.starts_with("HTTP/1.1 200") || r.starts_with("HTTP/1.0 200"),
        "shutdown: {r}"
    );
    if wait_exit(&mut child, Duration::from_secs(30)).is_none() {
        let _ = child.kill();
        panic!("kernel did not exit after /api/system/shutdown");
    }
}

/// The retry in `spawn_retrying_busy` is all that stands between a parallel
/// `fs::copy` and a red build, and the race it waits out is too narrow to
/// provoke by running tests. So stage the refusal directly: an image held open
/// for writing is exactly the state an inherited copy descriptor leaves it in.
/// Linux only — this asserts the refusal happens, and other systems are free
/// not to refuse.
#[cfg(target_os = "linux")]
#[test]
fn a_busy_executable_is_waited_out_rather_than_failed() {
    let source = Path::new("/bin/echo");
    if !source.exists() {
        eprintln!("skipping: this host has no /bin/echo to copy");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let exe = dir.path().join("busy");
    std::fs::copy(source, &exe).expect("copy /bin/echo");

    let writer = std::fs::OpenOptions::new()
        .write(true)
        .open(&exe)
        .expect("hold the image open for writing");

    // Precondition: while that descriptor is open, a plain spawn is refused.
    // Without it the rest of this test proves nothing.
    let refusal = Command::new(&exe)
        .stdout(Stdio::null())
        .spawn()
        .expect_err("a busy image must not exec");
    assert_eq!(
        refusal.kind(),
        std::io::ErrorKind::ExecutableFileBusy,
        "staged the wrong refusal: {refusal}"
    );

    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        drop(writer);
    });

    let mut cmd = Command::new(&exe);
    cmd.stdout(Stdio::null());
    let mut child = spawn_retrying_busy(&mut cmd);
    releaser.join().expect("releaser thread");
    assert!(
        child.wait().expect("wait for the spawned image").success(),
        "the image spawned by the retry did not run"
    );
}
