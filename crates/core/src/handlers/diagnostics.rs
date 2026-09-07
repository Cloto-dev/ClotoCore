//! Diagnostic report API — turns the failure a user just saw into text they can
//! paste into a GitHub issue.
//!
//! The report is composed here rather than in the UI. The version, the install
//! receipt and the log all live on this side, and a front end that composed it
//! would need two implementations (browser dashboard and Tauri shell) that
//! drift apart — while neither of them can read the log at all.
//!
//! Two levels, and both of them mask credentials:
//!
//! - [`Mode::Safe`] names the fields that may leave the machine (an allowlist)
//!   and masks every secret this kernel can enumerate out of each one. Paths
//!   under the user's home directory are shortened to `~`.
//! - [`Mode::Full`] keeps the log tail and the component stack at full length,
//!   for a report that needs them, and leaves paths intact. Credentials are
//!   still masked: they are useless for diagnosis and catastrophic to leak, so
//!   no level emits them.
//!
//! Masking is value-based first. The kernel collects the secrets it holds — the
//! live admin key, provider keys, secret-shaped MCP env values, its own
//! environment — and removes those exact strings wherever they appear, so a key
//! name never has to sit next to its value for the value to be caught. A
//! pattern pass then covers the credentials this kernel does not itself hold: a
//! bearer token minted elsewhere, a key inside a third-party server's output.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;

use super::ok_data;
use crate::{AppResult, AppState};

/// Trailing bytes of the log file to consider. The log rotates daily and has
/// been observed above 20 MB, so the tail is seeked to rather than read through.
const LOG_TAIL_BYTES: u64 = 512 * 1024;

/// Log lines carried at each level.
const SAFE_LOG_LINES: usize = 80;
const FULL_LOG_LINES: usize = 400;

/// Component-stack lines carried by the safe level.
const SAFE_STACK_LINES: usize = 40;

/// A string shorter than this is not masked as a secret. Short values collide
/// with ordinary words, and a report redacted into uselessness helps nobody.
const MIN_SECRET_LEN: usize = 8;

/// A name carrying one of these is treated as naming a credential.
const SECRET_NAME_MARKERS: [&str; 5] = ["KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL"];

/// Text that introduces a credential this kernel may not hold the value of.
/// Everything from the marker to the next delimiter is masked.
///
/// The scheme word is the marker, not the header name: `Authorization: ` would
/// match first and take `Bearer` as its value, leaving the token itself in the
/// report.
const CREDENTIAL_MARKERS: [&str; 10] = [
    "Bearer ",
    "bearer ",
    "Basic ",
    "X-API-Key: ",
    "x-api-key: ",
    "api_key=",
    "apikey=",
    "api-key=",
    "token=",
    "password=",
];

/// What replaces a masked value.
const MASK: &str = "«redacted»";

/// How much of the machine's own record the report carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Allowlisted fields only, secrets masked, home directory shortened.
    #[default]
    Safe,
    /// Log tail and stack left long, paths intact. Credentials still masked.
    Full,
}

impl Mode {
    const fn label(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Full => "full",
        }
    }

    const fn log_lines(self) -> usize {
        match self {
            Self::Safe => SAFE_LOG_LINES,
            Self::Full => FULL_LOG_LINES,
        }
    }
}

/// What the UI knows about the failure. Every field is optional: a report built
/// from nothing but the kernel's own state is still worth more than no report.
#[derive(Debug, Default, Deserialize)]
pub struct ReportRequest {
    /// The surface that failed, as the UI names it (e.g. "Marketplace install").
    #[serde(default)]
    pub context: Option<String>,
    /// The message the UI displayed.
    #[serde(default)]
    pub message: Option<String>,
    /// React component stack, when the ErrorBoundary caught the failure.
    #[serde(default)]
    pub component_stack: Option<String>,
    #[serde(default)]
    pub mode: Mode,
}

/// True when a variable name says its value is a credential.
fn is_secret_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    SECRET_NAME_MARKERS.iter().any(|m| upper.contains(m))
}

/// Removes secret values from text.
pub struct Redactor {
    /// Longest first, so masking a value that contains a shorter one does not
    /// leave the shorter one's tail behind.
    secrets: Vec<String>,
}

impl Redactor {
    /// Build from an explicit set of secret values.
    #[must_use]
    pub fn new(secrets: impl IntoIterator<Item = String>) -> Self {
        let mut secrets: Vec<String> = secrets
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| s.chars().count() >= MIN_SECRET_LEN)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        secrets.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        Self { secrets }
    }

    /// Collect every secret this kernel holds the value of.
    ///
    /// A source that fails to read is skipped rather than failing the report:
    /// the pattern pass still runs, and a report is more useful than an error.
    /// Note the asymmetry — a secret this misses is a secret that can reach the
    /// clipboard, so sources are added here, never trimmed for tidiness.
    pub async fn from_state(state: &AppState) -> Self {
        let mut found: BTreeSet<String> = BTreeSet::new();

        // The live admin key, not the boot-time snapshot: it is rotatable, and
        // the rotated value is the one that appears in logs.
        if let Ok(guard) = state.admin_api_key.read() {
            if let Some(key) = guard.as_ref() {
                found.insert(key.clone());
            }
        }

        if let Ok(keys) =
            sqlx::query_scalar::<_, Option<String>>("SELECT api_key FROM llm_providers")
                .fetch_all(&state.pool)
                .await
        {
            found.extend(keys.into_iter().flatten());
        }

        if let Ok(envs) = sqlx::query_scalar::<_, Option<String>>("SELECT env FROM mcp_servers")
            .fetch_all(&state.pool)
            .await
        {
            for raw in envs.into_iter().flatten() {
                let Ok(map) =
                    serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&raw)
                else {
                    continue;
                };
                for (name, value) in map {
                    if is_secret_name(&name) {
                        if let Some(text) = value.as_str() {
                            found.insert(text.to_string());
                        }
                    }
                }
            }
        }

        // This process's own environment: the kernel was started with the same
        // credentials it hands to the servers it spawns.
        for (name, value) in std::env::vars() {
            if is_secret_name(&name) {
                found.insert(value);
            }
        }

        Self::new(found)
    }

    /// Mask secrets in `text`, returning the result and how many replacements
    /// were made. The count is reported so a reader can tell "nothing secret
    /// was here" from "masking did not run".
    #[must_use]
    pub fn redact(&self, text: &str) -> (String, usize) {
        let mut out = text.to_string();
        let mut masked = 0usize;

        for secret in &self.secrets {
            if secret.is_empty() {
                continue;
            }
            let hits = out.matches(secret.as_str()).count();
            if hits > 0 {
                out = out.replace(secret.as_str(), MASK);
                masked += hits;
            }
        }

        let (out, pattern_hits) = mask_credential_patterns(&out);
        (out, masked + pattern_hits)
    }
}

/// Mask what follows a credential marker, for values this kernel does not hold.
fn mask_credential_patterns(text: &str) -> (String, usize) {
    let mut out = String::with_capacity(text.len());
    let mut masked = 0usize;

    for (i, line) in text.split_inclusive('\n').enumerate() {
        let _ = i;
        let mut rest = line;
        loop {
            // The earliest marker in what is left of the line.
            let hit = CREDENTIAL_MARKERS
                .iter()
                .filter_map(|m| rest.find(m).map(|at| (at, *m)))
                .min_by_key(|(at, _)| *at);

            let Some((at, marker)) = hit else {
                out.push_str(rest);
                break;
            };

            let value_start = at + marker.len();
            out.push_str(&rest[..value_start]);
            let tail = &rest[value_start..];
            let end = tail
                .find(|c: char| {
                    c.is_whitespace() || matches!(c, '"' | '\'' | ',' | '&' | '}' | ')' | ';')
                })
                .unwrap_or(tail.len());

            if tail[..end].chars().count() >= MIN_SECRET_LEN {
                out.push_str(MASK);
                masked += 1;
            } else {
                out.push_str(&tail[..end]);
            }
            rest = &tail[end..];
        }
    }

    (out, masked)
}

/// Strip ANSI SGR sequences. The kernel's own log is colourized, and the escape
/// codes survive a copy into a browser and land in the issue as noise.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        if chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        }
    }
    out
}

/// Replace the user's home directory with `~`. The home path carries the
/// account name, which is not needed to reproduce anything.
fn shorten_home(text: &str, home: Option<&str>) -> String {
    match home {
        Some(home) if home.len() > 1 => text.replace(home, "~"),
        _ => text.to_string(),
    }
}

/// The newest rotated log file. Names end in the date, so lexical order is
/// chronological order.
fn newest_log_file(data_dir: &Path) -> Option<PathBuf> {
    let mut best: Option<(String, PathBuf)> = None;
    for entry in std::fs::read_dir(data_dir).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("cloto-kernel.log") {
            continue;
        }
        if best.as_ref().is_none_or(|(best_name, _)| *best_name < name) {
            best = Some((name, entry.path()));
        }
    }
    best.map(|(_, path)| path)
}

/// The last `lines` lines of `path`, read by seeking to the tail.
fn read_log_tail(path: &Path, lines: usize) -> Vec<String> {
    let Ok(mut file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let len = file.metadata().map_or(0, |m| m.len());
    let start = len.saturating_sub(LOG_TAIL_BYTES);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut buf = Vec::new();
    if file.read_to_end(&mut buf).is_err() {
        return Vec::new();
    }

    let text = String::from_utf8_lossy(&buf);
    let mut all: Vec<&str> = text.lines().collect();
    // A seek into the middle of the file lands mid-line; that fragment is not a
    // log line and is dropped rather than reported as one.
    if start > 0 && !all.is_empty() {
        all.remove(0);
    }
    let from = all.len().saturating_sub(lines);
    all[from..].iter().map(|s| strip_ansi(s)).collect()
}

/// Compose the report body in the shape of `.github/ISSUE_TEMPLATE/bug_report.md`.
///
/// The template's first three sections are the user's to fill in; only
/// **Environment** and the collapsed evidence below it are machine-written, so
/// what the reporter is expected to add stays visible as empty structure.
#[allow(clippy::too_many_arguments)]
fn build_markdown(
    mode: Mode,
    request: &ReportRequest,
    app_version: &str,
    os: &str,
    arch: &str,
    engine: &str,
    receipt: Option<&crate::defender::footprint::Receipt>,
    log: &[String],
    masked: usize,
) -> String {
    let mut out = String::new();

    out.push_str("**Description**\n");
    match (&request.context, &request.message) {
        (Some(context), Some(message)) => {
            let _ = writeln!(out, "{context} failed: {message}");
        }
        (Some(context), None) => {
            let _ = writeln!(out, "{context} failed.");
        }
        (None, Some(message)) => {
            let _ = writeln!(out, "{message}");
        }
        (None, None) => out.push_str("<!-- what happened? -->\n"),
    }

    out.push_str("\n**Steps to Reproduce**\n1.\n2.\n3.\n");
    out.push_str("\n**Expected Behavior**\n\n");

    out.push_str("\n**Environment**\n");
    let _ = writeln!(out, "- ClotoCore version: {app_version}");
    let _ = writeln!(out, "- OS: {os} ({arch})");
    out.push_str("- Rust version:\n");
    let _ = writeln!(out, "- Install engine: {engine}");
    if let Some(receipt) = receipt {
        let secret_entries = receipt.entries.iter().filter(|e| e.secret).count();
        let _ = writeln!(
            out,
            "- Install receipt: {} entries ({} holding secrets, paths omitted), written by {} at {}",
            receipt.entries.len(),
            secret_entries,
            receipt.app_version,
            receipt.installed_at
        );
    } else {
        out.push_str("- Install receipt: none found\n");
    }

    if let Some(stack) = &request.component_stack {
        let shown: String = match mode {
            Mode::Safe => stack
                .lines()
                .take(SAFE_STACK_LINES)
                .collect::<Vec<_>>()
                .join("\n"),
            Mode::Full => stack.clone(),
        };
        out.push_str("\n<details><summary>Component stack</summary>\n\n```\n");
        out.push_str(shown.trim_end());
        out.push_str("\n```\n\n</details>\n");
    }

    if log.is_empty() {
        out.push_str("\n<!-- no kernel log was readable -->\n");
    } else {
        let _ = write!(
            out,
            "\n<details><summary>Kernel log — last {} lines</summary>\n\n```\n",
            log.len()
        );
        out.push_str(&log.join("\n"));
        out.push_str("\n```\n\n</details>\n");
    }

    let _ = writeln!(
        out,
        "\n<!-- Report level: {}. {masked} secret value(s) masked. Credentials are masked at every level. -->",
        mode.label()
    );

    out
}

/// POST /api/system/diagnostics — compose a pasteable report for a failure.
pub async fn report_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<ReportRequest>,
) -> AppResult<Json<serde_json::Value>> {
    super::check_auth(&state, &headers)?;

    let mode = request.mode;
    let redactor = Redactor::from_state(&state).await;

    let log_lines = newest_log_file(&state.data_dir)
        .map(|path| read_log_tail(&path, mode.log_lines()))
        .unwrap_or_default();

    let engine = crate::managers::installer::last_status().map_or_else(
        || "not probed".to_string(),
        |status| match status.version {
            Some(version) if status.is_ready() => version,
            Some(version) => format!("{version} (not usable: expected {})", status.expected),
            None => status.error.unwrap_or_else(|| "did not answer".to_string()),
        },
    );

    let receipt = crate::defender::footprint::load(&state.data_dir);

    let body = build_markdown(
        mode,
        &request,
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        &engine,
        receipt.as_ref(),
        &log_lines,
        0,
    );

    // Redaction runs over the assembled document, not each field, so a secret
    // that reached the report by any route is caught by one pass.
    let (body, masked) = redactor.redact(&body);
    let body = match mode {
        Mode::Safe => shorten_home(&body, home_dir().as_deref()),
        Mode::Full => body,
    };
    // The count is written after masking, so the note has to be corrected here.
    let body = body.replace(
        "0 secret value(s) masked",
        &format!("{masked} secret value(s) masked"),
    );

    ok_data(serde_json::json!({
        "markdown": body,
        "mode": mode.label(),
        "masked": masked,
        "log_lines": log_lines.len(),
    }))
}

/// The user's home directory as a string, when the platform names one.
fn home_dir() -> Option<String> {
    std::env::var("HOME")
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok())
        .filter(|h| !h.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_is_masked_wherever_it_appears_even_without_its_name() {
        let redactor = Redactor::new(["s3cr3t-value-abcdef".to_string()]);
        let (out, masked) = redactor.redact("connecting with s3cr3t-value-abcdef to the proxy");
        assert!(!out.contains("s3cr3t-value-abcdef"), "{out}");
        assert_eq!(masked, 1);
    }

    #[test]
    fn the_longest_secret_is_masked_first_so_no_tail_is_left_behind() {
        // The shorter value is a prefix of the longer one. Masking the short one
        // first would leave "-suffix" behind in the report.
        let redactor = Redactor::new(["abcdefgh".to_string(), "abcdefgh-suffix".to_string()]);
        let (out, _) = redactor.redact("value=abcdefgh-suffix");
        assert!(!out.contains("suffix"), "{out}");
    }

    #[test]
    fn a_short_value_is_not_treated_as_a_secret() {
        // Masking a value this short would redact ordinary words out of the log.
        let redactor = Redactor::new(["abc".to_string()]);
        let (out, masked) = redactor.redact("abc appears in many ordinary words");
        assert_eq!(out, "abc appears in many ordinary words");
        assert_eq!(masked, 0);
    }

    #[test]
    fn a_bearer_token_this_kernel_does_not_hold_is_still_masked() {
        let redactor = Redactor::new(Vec::<String>::new());
        let (out, masked) =
            redactor.redact("GET /v1/models Authorization: Bearer sk-not-ours-1234\n");
        assert!(!out.contains("sk-not-ours-1234"), "{out}");
        assert!(masked >= 1);
    }

    #[test]
    fn masking_stops_at_the_delimiter_and_keeps_the_rest_of_the_line() {
        let redactor = Redactor::new(Vec::<String>::new());
        let (out, _) = redactor.redact("url?api_key=abcdefghijkl&model=gpt\n");
        assert!(out.contains("&model=gpt"), "{out}");
        assert!(!out.contains("abcdefghijkl"), "{out}");
    }

    #[test]
    fn the_full_level_masks_credentials_too() {
        // The difference between the levels is length and paths, never secrets.
        let redactor = Redactor::new(["s3cr3t-value-abcdef".to_string()]);
        let report = build_markdown(
            Mode::Full,
            &ReportRequest {
                message: Some("failed with s3cr3t-value-abcdef".to_string()),
                mode: Mode::Full,
                ..ReportRequest::default()
            },
            "0.0.0-test",
            "linux",
            "x86_64",
            "0.0.0-test",
            None,
            &[],
            0,
        );
        let (out, masked) = redactor.redact(&report);
        assert!(!out.contains("s3cr3t-value-abcdef"), "{out}");
        assert_eq!(masked, 1);
    }

    #[test]
    fn the_safe_level_shortens_the_home_directory() {
        let out = shorten_home("/Users/someone/Library/data", Some("/Users/someone"));
        assert_eq!(out, "~/Library/data");
    }

    #[test]
    fn ansi_colour_codes_are_stripped_from_log_lines() {
        let out =
            strip_ansi("\u{1b}[2m2026-09-07T00:00:00Z\u{1b}[0m \u{1b}[32m INFO\u{1b}[0m ready");
        assert_eq!(out, "2026-09-07T00:00:00Z  INFO ready");
    }

    #[test]
    fn the_report_keeps_the_sections_the_reporter_has_to_fill_in() {
        let report = build_markdown(
            Mode::Safe,
            &ReportRequest::default(),
            "0.0.0-test",
            "macos",
            "aarch64",
            "0.0.0-test",
            None,
            &["one line".to_string()],
            0,
        );
        for heading in [
            "**Description**",
            "**Steps to Reproduce**",
            "**Expected Behavior**",
            "**Environment**",
        ] {
            assert!(report.contains(heading), "missing {heading} in:\n{report}");
        }
        assert!(report.contains("- ClotoCore version: 0.0.0-test"));
    }

    #[test]
    fn a_name_that_says_credential_is_recognized_case_insensitively() {
        assert!(is_secret_name("DISCORD_BOT_TOKEN"));
        assert!(is_secret_name("cloto_api_key"));
        assert!(is_secret_name("SomeSecretValue"));
        assert!(!is_secret_name("CSCHEDULER_DB_PATH"));
    }
}
