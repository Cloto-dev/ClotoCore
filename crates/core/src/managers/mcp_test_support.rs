//! Test-only support for the python-mock MCP servers used by the `mcp_client`
//! tests: one readiness gate every mock-backed connect goes through.
//!
//! # The race this closes
//!
//! [`McpClient::connect`] spawns the child and sends its first request in the
//! same breath. `StdioTransport::start` returns once the process *exists*, not
//! once it can *read*, and `negotiate` writes `server/discover` immediately
//! after. The request is never lost — it waits in the stdin pipe — but the
//! timeout clock starts at the write, so the whole of interpreter startup is
//! charged to the first request's window. On the `windows-latest` runner a cold
//! `python3 -c` has been measured at ~4 s (job 99374988960 against its green
//! re-run at the same commit), and every mock-backed test in the module starts
//! its own interpreter, concurrently, on a runner that is already saturated by
//! the rest of the suite. When startup outruns the window the test dies with
//! `MCP Request timed out` on `initialize` / `server/discover` — three tests on
//! one pull request and six on another, both on 2026-09-04, every re-run green.
//!
//! # The gate
//!
//! [`connect_mock`] never hands a mock source straight to the client. It first
//! runs the *same program* under the *same interpreter* with stdin closed: the
//! program announces itself on **stderr** — stdout is the JSON-RPC channel —
//! the instant it is about to read stdin, then reads EOF and exits on its own.
//! Waiting for that sentinel pays the cold start (page cache, interpreter and
//! `site` init, compiling the `-c` source) *before* any request window is open,
//! so the connect that follows starts from a warm interpreter.
//!
//! What the gate does **not** do is gate the connected child itself: the client
//! offers no hook between spawn and the first request, and adding one would be
//! a production change made for the tests. The gate therefore removes the cost
//! that makes the race lose, and [`MOCK_REQUEST_TIMEOUT_SECS`] covers what is
//! left — see its docs for the part of the window that is *not* ours to widen.
//!
//! Two side effects worth having: a mock with a syntax error now fails with the
//! interpreter's own message instead of an opaque request timeout, and the
//! measured readiness latency is printed, so the next time this gets slow on a
//! runner the number is already in the log.

use super::mcp_client::{McpClient, McpNotification, NegotiatedProtocol, DEFAULT_MCP_LOG_LEVEL};
use anyhow::Result;
use std::collections::HashMap;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

/// Line a mock writes to stderr once it is about to read stdin.
///
/// Chosen to look like nothing an MCP server would emit on its own, because the
/// stderr bridge forwards it to the notification channel like any other line —
/// tests that grade forwarded stderr skip it by exact match.
pub(crate) const MOCK_READY_SENTINEL: &str = "CLOTO_MOCK_READY";

/// How long [`connect_mock`] waits for the sentinel before failing the test.
///
/// Deliberately far past any plausible interpreter start: this bound is not a
/// budget the tests are meant to sit near, it is the point at which "python is
/// slow" becomes "python is broken" and the test should say so instead of
/// hanging until the harness gives up.
const READY_TIMEOUT_SECS: u64 = 60;

/// Request timeout the mock connects ask for.
///
/// Sized so that no window a mock test grades can expire on interpreter
/// startup, even if the readiness gate above somehow bought nothing: 60 s is
/// more than an order of magnitude past the ~4 s cold start measured on the
/// Windows runner.
///
/// **It does not widen everything.** The `server/discover` probe window is
/// `min(request_timeout, DISCOVER_PROBE_TIMEOUT_SECS)` — production caps it at
/// 10 s so a silent server costs one short probe rather than a full request
/// window — so for the tests that grade the probe *answer* this constant buys
/// nothing beyond 10 s. That cap is exactly why the readiness gate is the
/// primary mechanism here and this number is the backstop: raising it further
/// would not make those tests any safer.
const MOCK_REQUEST_TIMEOUT_SECS: u64 = 60;

/// Stream idle timeout for mock connects. No mock streams; the value only has
/// to exist.
const MOCK_STREAM_IDLE_TIMEOUT_SECS: u64 = 5;

/// Notification channel depth handed to a mock connect. Room for the readiness
/// sentinel, the mock's own stderr, and its notifications without the bridge
/// (which drops on a full channel) losing what a test is polling for.
const NOTIFICATION_BUFFER: usize = 16;

/// A connected mock server: the client, what the connect negotiated, and the
/// notification channel kept alive for the test to poll.
pub(crate) struct MockServer {
    pub(crate) client: McpClient,
    pub(crate) negotiated: NegotiatedProtocol,
    pub(crate) notifications: mpsc::Receiver<McpNotification>,
}

/// The interpreter a mock connect will actually spawn.
///
/// Mirrors `mcp_transport::resolve_python_command`, which redirects
/// `python`/`python3` to the managed venv when there is one. The readiness gate
/// has to warm the binary the transport will start, not merely one with the
/// same name.
fn python_command() -> String {
    super::mcp_venv::resolve_venv_python().map_or_else(
        || "python3".to_string(),
        |p| p.to_string_lossy().into_owned(),
    )
}

/// Whether the interpreter the mocks need is runnable, printing the standard
/// skip line if it is not. Keeps minimal environments green without pretending
/// the coverage was there.
pub(crate) fn python3_available(test_name: &str) -> bool {
    if std::process::Command::new(python_command())
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("skipping {test_name}: python3 not found");
        return false;
    }
    true
}

/// Prefix a mock body with its readiness announcement.
///
/// `python -c` compiles the whole source before running a line of it, so a
/// sentinel at the top is emitted after everything expensive about starting up
/// — interpreter init, `site`, compiling this source — and only microseconds
/// before the body reaches its `sys.stdin.readline()` loop.
fn mock_program(body: &str) -> String {
    format!(
        "import sys\n\
         sys.stderr.write('{MOCK_READY_SENTINEL}\\n')\n\
         sys.stderr.flush()\n\
         {body}"
    )
}

/// Run `program` with stdin closed and wait for it to announce readiness.
///
/// Panics — this is the gate, and a mock that never reaches its read loop is a
/// failure the test must report, not absorb:
///
/// * spawn failed → the OS error, not a later timeout;
/// * exited without the sentinel → whatever it wrote to stderr, which for a
///   malformed mock is the interpreter's own `SyntaxError`;
/// * still silent after [`READY_TIMEOUT_SECS`] → said plainly, with the bound.
async fn await_mock_ready(server_id: &str, program: &str) {
    let started = Instant::now();
    let mut cmd = Command::new(python_command());
    cmd.arg("-c")
        .arg(program)
        // stdin closed: the sentinel is written before the read loop, so the
        // loop then sees EOF and the warm-up exits on its own.
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        // Same env the transport sets, so this warms the same startup path.
        .env("PYTHONUNBUFFERED", "1")
        .kill_on_drop(true);
    // Matches the transport: no console window flashing on the Windows runner.
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd
        .spawn()
        .unwrap_or_else(|e| panic!("{server_id}: could not start the readiness warm-up: {e}"));
    let stderr = child
        .stderr
        .take()
        .unwrap_or_else(|| panic!("{server_id}: readiness warm-up has no stderr pipe"));

    let mut seen = Vec::new();
    let mut lines = BufReader::new(stderr).lines();
    let outcome = tokio::time::timeout(Duration::from_secs(READY_TIMEOUT_SECS), async {
        while let Ok(Some(line)) = lines.next_line().await {
            if line.trim() == MOCK_READY_SENTINEL {
                return true;
            }
            seen.push(line);
        }
        false
    })
    .await;

    let elapsed = started.elapsed();
    // It has almost certainly exited already; this reaps it either way.
    let _ = child.kill().await;

    let announced = match outcome {
        Ok(announced) => announced,
        Err(deadline) => panic!(
            "{server_id}: the mock never announced readiness within \
             {READY_TIMEOUT_SECS}s ({deadline}). Its stderr so far was: {seen:?}"
        ),
    };
    assert!(
        announced,
        "{server_id}: the mock's stderr ended without a readiness announcement after \
         {elapsed:?}. Its stderr was: {seen:?}"
    );
    eprintln!("{server_id}: mock ready in {elapsed:?}");
}

/// Wait for the mock to prove it can start, then connect to a fresh instance of
/// it (era preference `auto`).
///
/// `body` is the mock's python source *without* the readiness preamble — this
/// adds it, so no test can connect to a mock that has not been through the
/// gate. The returned [`MockServer`] owns the notification receiver: dropping it
/// would close the channel and silently strand the stderr bridge.
pub(crate) async fn connect_mock(server_id: &str, body: &str) -> Result<MockServer> {
    let program = mock_program(body);
    await_mock_ready(server_id, &program).await;

    let (notif_tx, notifications) = mpsc::channel(NOTIFICATION_BUFFER);
    let (client, negotiated) = McpClient::connect(
        server_id,
        "python3",
        &["-c".to_string(), program],
        &HashMap::new(),
        notif_tx,
        MOCK_REQUEST_TIMEOUT_SECS,
        MOCK_STREAM_IDLE_TIMEOUT_SECS,
        None,
        0,
        "",
        &[],
        DEFAULT_MCP_LOG_LEVEL,
        None,
    )
    .await?;

    Ok(MockServer {
        client,
        negotiated,
        notifications,
    })
}

#[cfg(test)]
mod tests {
    /// The gate only holds if every mock-backed connect goes through it, and a
    /// comment cannot enforce that: the next test to spawn a python mock will
    /// copy whichever neighbour it reads first. Scanning the test module for a
    /// direct `McpClient::connect` keeps the copy-paste path pointed at
    /// [`super::connect_mock`].
    ///
    /// `connect_http` is deliberately not matched — an HTTP-transport test has
    /// no interpreter to wait for.
    #[test]
    fn no_test_connects_a_mock_without_the_readiness_gate() {
        let src = include_str!("mcp_client.rs");
        let (_, test_module) = src
            .split_once("\nmod tests {")
            .expect("mcp_client.rs must still have a `mod tests`");
        assert!(
            !test_module.contains("McpClient::connect("),
            "a test in mcp_client.rs calls `McpClient::connect` directly. Mock-backed \
             connects must go through `mcp_test_support::connect_mock`, which waits for \
             the mock to announce readiness before the client sends anything — a direct \
             connect races interpreter startup and fails on CI as `MCP Request timed \
             out`. See this module's docs."
        );
    }
}
