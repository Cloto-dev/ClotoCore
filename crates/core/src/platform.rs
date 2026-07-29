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

/// Quote one argument so the child splits it back out as one argument.
///
/// `std::process::Command` already does this, so only the one call that cannot
/// use it needs it spelled out: the elevation prompt on Windows goes through
/// PowerShell's `Start-Process -Verb RunAs` (`spawn_elevated_and_wait`), and
/// `-ArgumentList` is not an argv — PowerShell joins its elements with spaces
/// into a single command line and quotes nothing. Any element containing a
/// space therefore arrives at the child as two arguments.
///
/// That is not hypothetical: an elevated uninstall of a per-machine install
/// passes `--root C:\Program Files\ClotoCore` (`DEFENDER_DESIGN.md` §7 sends
/// the containment roots on the command line), the helper received `--root
/// C:\Program` followed by a stray `Files\ClotoCore`, and argument parsing
/// rejected it — before the point where a report is written, so the uninstall
/// removed nothing and said nothing.
///
/// The rules satisfied here are the ones the child's runtime applies when it
/// splits its command line apart again (`CommandLineToArgvW`): whitespace
/// separates arguments unless quoted, `\"` is a literal quote, and a run of
/// backslashes is doubled when it precedes a quote — including the closing one,
/// which a lone trailing backslash would otherwise escape.
///
/// Lives here, outside the `#[cfg(windows)]` module, because it is pure string
/// handling and its tests are the only evidence that it is right: on the
/// Windows side of the `#[cfg]` they would run on one CI runner, which is
/// exactly where the original bug survived.
#[must_use]
pub fn win_argv_quote(arg: &str) -> String {
    // Left alone when there is nothing to protect. Quoting unconditionally
    // would also parse, but it makes every generated command line — the thing
    // one reads in a log or a process listing while debugging this path —
    // harder to compare against the arguments it was built from.
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }

    let mut quoted = String::with_capacity(arg.len() + 2);
    quoted.push('"');
    let mut backslashes = 0usize;
    for ch in arg.chars() {
        match ch {
            // Only meaningful once what follows is known, so it is counted, not
            // emitted.
            '\\' => backslashes += 1,
            '"' => {
                // Each pending backslash has to survive as a backslash (hence
                // doubled), and one more is needed to make this quote literal.
                push_backslashes(&mut quoted, backslashes * 2 + 1);
                backslashes = 0;
                quoted.push('"');
            }
            _ => {
                push_backslashes(&mut quoted, backslashes);
                backslashes = 0;
                quoted.push(ch);
            }
        }
    }
    // A trailing run precedes the closing quote, so it is doubled too.
    push_backslashes(&mut quoted, backslashes * 2);
    quoted.push('"');
    quoted
}

fn push_backslashes(out: &mut String, count: usize) {
    for _ in 0..count {
        out.push('\\');
    }
}

/// Split a Windows command line back into arguments, the way a child process
/// does.
///
/// Test support, and deliberately the *inverse* of [`win_argv_quote`] rather
/// than a restatement of it: comparing against an expected quoted string only
/// proves the output looks the way whoever wrote the assertion imagined, while
/// a round-trip states the property that actually matters — the child receives
/// the arguments it was given. Lives next to the function it checks so the
/// uninstall handoff (`defender::uninstall`) can make that claim about its real
/// argument list too.
///
/// Takes the arguments portion only: the program name at the front of a real
/// command line is parsed by different rules, and PowerShell's `-ArgumentList`
/// never contains it.
#[cfg(test)]
pub(crate) fn parse_windows_arguments(command_line: &str) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    // An argument can legitimately be empty (`""`), so emptiness cannot decide
    // whether one has been started.
    let mut started = false;
    let mut backslashes = 0usize;

    for ch in command_line.chars() {
        match ch {
            '\\' => {
                backslashes += 1;
                started = true;
            }
            '"' => {
                push_backslashes(&mut current, backslashes / 2);
                if backslashes % 2 == 1 {
                    current.push('"');
                } else {
                    in_quotes = !in_quotes;
                }
                backslashes = 0;
                started = true;
            }
            ' ' | '\t' if !in_quotes => {
                push_backslashes(&mut current, backslashes);
                backslashes = 0;
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            _ => {
                push_backslashes(&mut current, backslashes);
                backslashes = 0;
                current.push(ch);
                started = true;
            }
        }
    }
    push_backslashes(&mut current, backslashes);
    if started {
        args.push(current);
    }
    args
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

    #[test]
    fn an_argument_containing_a_space_is_quoted() {
        // The case that shipped broken: both of these are roots of a
        // per-machine Windows install.
        assert_eq!(
            win_argv_quote(r"C:\Program Files\ClotoCore"),
            r#""C:\Program Files\ClotoCore""#
        );
        assert_eq!(
            win_argv_quote(r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs"),
            r#""C:\ProgramData\Microsoft\Windows\Start Menu\Programs""#
        );
        assert_eq!(win_argv_quote("tab\there"), "\"tab\there\"");
    }

    #[test]
    fn an_argument_without_whitespace_is_left_as_it_was() {
        // Quoting what needs no quoting still parses, but the generated command
        // line stops resembling the arguments it was built from.
        assert_eq!(win_argv_quote("purge-exec"), "purge-exec");
        assert_eq!(win_argv_quote("--root"), "--root");
        assert_eq!(
            win_argv_quote(r"C:\ProgramData\ClotoCore"),
            r"C:\ProgramData\ClotoCore"
        );
        // A trailing backslash needs no protection while there is no closing
        // quote for it to escape.
        assert_eq!(win_argv_quote(r"C:\ClotoCore\"), r"C:\ClotoCore\");
    }

    #[test]
    fn an_embedded_quote_is_escaped() {
        assert_eq!(win_argv_quote(r#"a "b" c"#), r#""a \"b\" c""#);
        // Backslashes before a quote are doubled, so they stay backslashes
        // instead of escaping the escape.
        assert_eq!(win_argv_quote(r"a b\"), r#""a b\\""#);
        assert_eq!(win_argv_quote(r#"a b\""#), r#""a b\\\"""#);
    }

    #[test]
    fn a_trailing_backslash_cannot_escape_the_closing_quote() {
        // Without the doubling the command line ends `...ClotoCore\"`, the
        // closing quote is consumed as a literal, and the next argument is
        // swallowed into this one.
        assert_eq!(
            win_argv_quote(r"C:\Program Files\ClotoCore\"),
            r#""C:\Program Files\ClotoCore\\""#
        );
        assert_eq!(
            win_argv_quote(r"C:\Program Files\ClotoCore\\"),
            r#""C:\Program Files\ClotoCore\\\\""#
        );
    }

    #[test]
    fn an_empty_argument_stays_one_argument() {
        assert_eq!(win_argv_quote(""), r#""""#);
    }

    #[test]
    fn quoting_round_trips_through_a_command_line() {
        // The property the child depends on, stated end to end: quote every
        // argument, join them the way PowerShell's -ArgumentList does (spaces,
        // no quoting of its own), and the split must return the original list.
        let args: Vec<String> = [
            "purge-exec",
            "--plan",
            r"C:\Users\Dr X\AppData\Local\Temp\clotocore-uninstall-42\purge-plan.json",
            "--pid",
            "4242",
            "--root",
            r"C:\Program Files\ClotoCore",
            "--root",
            r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs",
            r"C:\one space\trailing\",
            r"C:\two spaces here\trailing\\",
            r#"quote"in the middle"#,
            r#""quoted at the edges""#,
            "",
            "tab\tand space",
            "plain",
        ]
        .iter()
        .map(|a| (*a).to_string())
        .collect();

        let command_line = args
            .iter()
            .map(|a| win_argv_quote(a))
            .collect::<Vec<_>>()
            .join(" ");

        assert_eq!(
            parse_windows_arguments(&command_line),
            args,
            "command line was: {command_line}"
        );
    }

    #[test]
    fn the_parser_splits_an_unquoted_command_line_the_way_windows_does() {
        // The round-trip above is only evidence if the parser is not simply
        // agreeing with the quoter. This pins it to the failure that was
        // actually observed: unquoted, `C:\Program Files\ClotoCore` is two
        // arguments, and the second one is what the helper rejected.
        assert_eq!(
            parse_windows_arguments(r"--root C:\Program Files\ClotoCore"),
            vec![
                "--root".to_string(),
                r"C:\Program".to_string(),
                r"Files\ClotoCore".to_string(),
            ]
        );
        assert_eq!(
            parse_windows_arguments("  a\t\tb "),
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(parse_windows_arguments(""), Vec::<String>::new());
        assert_eq!(parse_windows_arguments(r#"a "" b"#), ["a", "", "b"]);
    }
}
