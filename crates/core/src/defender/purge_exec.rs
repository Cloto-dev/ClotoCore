//! Purge executor — the plan-bound half of the uninstall path
//! (DEFENDER_DESIGN.md §7, safety invariant §8.5).
//!
//! The plan is the capability boundary: this module removes exactly what a
//! plan lists, and it has no enumeration logic of its own. It is not, however,
//! a blind applier. A plan arrives as a *file* — written by an earlier
//! process, possibly by a different binary, and read back from the temp
//! directory the detached helper runs from — so every floor the planner
//! applied is applied again here, to the values actually read: a path that is
//! relative, a filesystem root, or carries a `..` component is refused rather
//! than removed, and a plan whose format version this binary does not execute
//! removes nothing at all.
//!
//! Three properties of the removal itself are deliberate:
//!
//! - **Idempotent.** An entry that is already gone is a success, not an
//!   error. Re-running a partially completed uninstall must converge.
//! - **Symlinks are never followed.** A link is unlinked; whatever it points
//!   at is left alone. The plan measured the link, so removing the target
//!   would delete bytes the user was never shown.
//! - **A type change refuses.** If the plan says directory and the disk now
//!   holds a file, the disk changed after enumeration — precisely the state in
//!   which deleting is least safe.
//!
//! One failure never aborts the run: a partially completed uninstall is still
//! worth finishing, and the report says exactly what happened to every entry.

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};

use crate::defender::purge::{
    self, is_filesystem_root, order_for_removal, tier_label, PlanRequest, PurgeEntry, PurgeKind,
    PurgePlan, PurgeTier, PLAN_VERSION,
};

/// A plan is a list of paths, not a payload. Anything larger than this is not
/// a plan this binary wrote, and it is read from disk before any of it is
/// trusted — so the bound comes first.
const MAX_PLAN_BYTES: u64 = 8 * 1024 * 1024;

/// How long the detached helper waits for the kernel that spawned it to exit
/// (§7 choreography). Matches the updater's `execute_swap` handoff window.
const PARENT_EXIT_TIMEOUT: Duration = Duration::from_secs(30);

// ── Report ──

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryOutcome {
    /// The entry was present and is now gone.
    Removed,
    /// Nothing was there. Idempotent success — an uninstall that is re-run,
    /// or that follows a manual cleanup, must not report an error.
    Absent,
    /// The executor's own safety floor rejected the entry; nothing was
    /// touched.
    Refused { reason: String },
    /// Removal was attempted and failed. The item is still on disk.
    Failed { error: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryReport {
    pub id: String,
    pub kind: PurgeKind,
    /// Path, or service name for `Service` entries — whatever the plan named.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub outcome: EntryOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PurgeReport {
    /// Format version of the plan that was executed (not necessarily the one
    /// this binary executes — see `execute`).
    pub plan_version: u32,
    /// Version of the binary that generated the plan.
    pub app_version: String,
    pub tier: PurgeTier,
    pub entries: Vec<EntryReport>,
}

impl PurgeReport {
    #[must_use]
    pub fn removed(&self) -> usize {
        self.count(|o| matches!(o, EntryOutcome::Removed))
    }

    #[must_use]
    pub fn absent(&self) -> usize {
        self.count(|o| matches!(o, EntryOutcome::Absent))
    }

    #[must_use]
    pub fn refused(&self) -> usize {
        self.count(|o| matches!(o, EntryOutcome::Refused { .. }))
    }

    #[must_use]
    pub fn failed(&self) -> usize {
        self.count(|o| matches!(o, EntryOutcome::Failed { .. }))
    }

    fn count(&self, pred: impl Fn(&EntryOutcome) -> bool) -> usize {
        self.entries.iter().filter(|e| pred(&e.outcome)).count()
    }
}

// ── Execution ──

/// Execute `plan`, removing only what it lists.
///
/// Never panics and never returns early: every entry gets an outcome, so the
/// report is a complete account of the run even when parts of it failed.
#[must_use]
pub fn execute(plan: &PurgePlan) -> PurgeReport {
    let mut report = PurgeReport {
        plan_version: plan.plan_version,
        app_version: plan.app_version.clone(),
        tier: plan.tier,
        entries: Vec::with_capacity(plan.entries.len()),
    };

    // A plan written in a format this binary does not execute is not
    // partially honoured: field meanings are exactly what could have changed,
    // and a misread field here deletes the wrong thing.
    if plan.plan_version != PLAN_VERSION {
        let reason = format!(
            "plan version mismatch: plan is v{}, this binary executes v{PLAN_VERSION}",
            plan.plan_version
        );
        report.entries = plan
            .entries
            .iter()
            .map(|entry| {
                describe(
                    entry,
                    EntryOutcome::Refused {
                        reason: reason.clone(),
                    },
                )
            })
            .collect();
        return report;
    }

    // Deregistrations first, then deepest paths first (the planner's own
    // ordering rule — a launchd job cannot be unloaded once its plist is
    // gone).
    for entry in order_for_removal(plan.entries.clone()) {
        // The tier the plan declares is a floor like any other, not a caption.
        // The planner filters by it (`purge.rs`), and the report prints it — so
        // an entry wider than the declared scope would be removed while the
        // header said "application only".
        let outcome = if plan.tier.includes(entry.tier) {
            execute_entry(&entry)
        } else {
            refuse(format!(
                "entry is tier {} but the plan declares tier {}",
                entry.tier.level(),
                plan.tier.level()
            ))
        };
        report.entries.push(describe(&entry, outcome));
    }
    report
}

fn describe(entry: &PurgeEntry, outcome: EntryOutcome) -> EntryReport {
    let target = match entry.kind {
        // The service branch ignores the plan's name by design, so echoing it
        // would report that an arbitrarily named service was deregistered. Only
        // this installation's own registration can be, so say that instead.
        PurgeKind::Service => Some("this installation's own service registration".to_string()),
        _ => entry.path.clone().or_else(|| entry.name.clone()),
    };
    EntryReport {
        id: entry.id.clone(),
        kind: entry.kind,
        target,
        outcome,
    }
}

fn refuse(reason: impl Into<String>) -> EntryOutcome {
    EntryOutcome::Refused {
        reason: reason.into(),
    }
}

fn execute_entry(entry: &PurgeEntry) -> EntryOutcome {
    match entry.kind {
        PurgeKind::File | PurgeKind::Dir => match checked_path(entry.path.as_deref()) {
            Err(reason) => refuse(reason),
            Ok(path) => remove_path(&path, entry.kind),
        },
        PurgeKind::Service => match non_empty(entry.name.as_deref()) {
            None => refuse("service entry carries no service name"),
            // `uninstall_service` deregisters the one service this binary
            // installs; the plan cannot ask for an arbitrary unit.
            Some(_) => match crate::platform::uninstall_service() {
                Ok(true) => EntryOutcome::Removed,
                Ok(false) => EntryOutcome::Absent,
                Err(e) => EntryOutcome::Failed {
                    error: format!("{e:#}"),
                },
            },
        },
        PurgeKind::Registry => remove_registry(entry.path.as_deref()),
    }
}

/// Re-apply the planner's path floor to a value that has been through a file.
///
/// The planner refuses these too, but a plan is an input: it can be edited,
/// truncated, or produced by another version. The executor is the last place
/// where a bad path is still just a string.
fn checked_path(raw: Option<&str>) -> Result<PathBuf, String> {
    let Some(raw) = non_empty(raw) else {
        return Err("entry carries no path".to_string());
    };
    let path = PathBuf::from(raw);
    if !path.is_absolute() {
        return Err(format!(
            "path is not absolute: {raw} (the helper runs from a temp directory, where a relative \
             path resolves somewhere else)"
        ));
    }
    if is_filesystem_root(&path) {
        return Err(format!("path is a filesystem root: {raw}"));
    }
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(format!("path contains a parent-directory component: {raw}"));
    }
    Ok(path)
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.trim().is_empty())
}

fn remove_path(path: &Path, kind: PurgeKind) -> EntryOutcome {
    // `symlink_metadata` deliberately: `metadata` would report the *target* of
    // a symlink and take the branch that deletes it.
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return EntryOutcome::Absent,
        Err(e) => {
            return EntryOutcome::Failed {
                error: format!("cannot inspect {}: {e}", path.display()),
            }
        }
    };

    if meta.file_type().is_symlink() {
        // Unlink the link itself, whatever the plan called it. The plan sized
        // the link, so following it would delete bytes nobody approved.
        return match remove_symlink(path) {
            Ok(()) => EntryOutcome::Removed,
            Err(e) => EntryOutcome::Failed {
                error: format!("cannot remove symlink {}: {e}", path.display()),
            },
        };
    }

    let result = match kind {
        PurgeKind::Dir if meta.is_dir() => std::fs::remove_dir_all(path),
        PurgeKind::File if meta.is_file() => std::fs::remove_file(path),
        _ => {
            return refuse(format!(
                "type changed since the plan was generated: {} is not {}",
                path.display(),
                if matches!(kind, PurgeKind::Dir) {
                    "a directory"
                } else {
                    "a regular file"
                }
            ))
        }
    };

    match result {
        Ok(()) => EntryOutcome::Removed,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => EntryOutcome::Absent,
        Err(e) => EntryOutcome::Failed {
            error: format!("cannot remove {}: {e}", path.display()),
        },
    }
}

/// Remove a symlink without touching its target. On Windows a directory
/// symlink is a reparse point that `remove_file` refuses, so fall back to
/// `remove_dir` — which, for a link, unlinks rather than recurses.
fn remove_symlink(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) => std::fs::remove_dir(path).map_err(|_| e),
    }
}

/// The only registry location an uninstall may delete: one application's own
/// entry under the per-machine or per-user uninstall list.
///
/// `reg delete /f` is recursive and the helper runs elevated (§7), so without
/// this an edited plan could ask for `HKLM\Software` and be obeyed. `Service`
/// closes exactly this hole by ignoring the plan's unit name; the registry
/// branch has to close it by checking the key, because unlike the service
/// there is more than one legitimate key (`purge.rs` emits four: `HKLM` and
/// `HKCU` × `ClotoCore` and the pre-rename `cloto-system`).
const UNINSTALL_KEY_PREFIX: [&str; 5] = [
    "Software",
    "Microsoft",
    "Windows",
    "CurrentVersion",
    "Uninstall",
];

/// Is `key` exactly `HK(LM|CU)\Software\Microsoft\Windows\CurrentVersion\
/// Uninstall\<one component>`? Case-insensitive, because registry paths are.
fn is_uninstall_entry_key(key: &str) -> bool {
    // Only backslashes separate registry components. Accepting `/` would let a
    // single component smuggle a deeper path past the length check below.
    if key.contains('/') {
        return false;
    }
    let mut parts = key.split('\\');

    let Some(hive) = parts.next() else {
        return false;
    };
    if !["HKLM", "HKCU"]
        .iter()
        .any(|h| h.eq_ignore_ascii_case(hive))
    {
        return false;
    }

    let rest: Vec<&str> = parts.collect();
    // The five fixed components plus exactly one leaf: no shorter (that would
    // be the whole uninstall list), no longer, no empty component from a
    // doubled or trailing separator.
    if rest.len() != UNINSTALL_KEY_PREFIX.len() + 1 {
        return false;
    }
    if rest.iter().any(|c| c.trim().is_empty()) {
        return false;
    }
    if !UNINSTALL_KEY_PREFIX
        .iter()
        .zip(&rest)
        .all(|(expected, actual)| expected.eq_ignore_ascii_case(actual))
    {
        return false;
    }
    // A leaf is one key name, never a pattern.
    let leaf = rest[UNINSTALL_KEY_PREFIX.len()];
    !leaf.contains('*') && !leaf.contains('?')
}

fn remove_registry(key: Option<&str>) -> EntryOutcome {
    let Some(key) = non_empty(key) else {
        return refuse("registry entry carries no key path");
    };
    // Checked before the platform gate so the boundary is testable — and
    // reviewable — on every platform, not only where it can be exercised.
    if !is_uninstall_entry_key(key) {
        return refuse(format!(
            "registry key is outside the uninstall list: {key} (only \
             HKLM|HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\<name> may be \
             removed)"
        ));
    }
    if !cfg!(windows) {
        return refuse("registry keys exist only on Windows");
    }
    match crate::platform::delete_registry_key(key) {
        Ok(true) => EntryOutcome::Removed,
        Ok(false) => EntryOutcome::Absent,
        Err(e) => EntryOutcome::Failed {
            error: format!("{e:#}"),
        },
    }
}

// ── Rendering ──

/// Human-readable rendering of a report. Kept pure, like the plan's own
/// renderer, so the account the user is given is itself testable.
#[must_use]
pub fn render_text(report: &PurgeReport) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    let _ = writeln!(
        out,
        "ClotoCore uninstall report (plan format v{}, generated by v{})",
        report.plan_version, report.app_version
    );
    let _ = writeln!(
        out,
        "Scope tier:     {} ({})",
        report.tier.level(),
        tier_label(report.tier)
    );
    let _ = writeln!(out);

    if report.entries.is_empty() {
        let _ = writeln!(out, "  Nothing to remove.");
        return out;
    }

    for entry in &report.entries {
        let target = entry
            .target
            .clone()
            .unwrap_or_else(|| "<unnamed>".to_string());
        let (label, detail) = match &entry.outcome {
            EntryOutcome::Removed => ("removed", String::new()),
            EntryOutcome::Absent => ("absent ", String::new()),
            EntryOutcome::Refused { reason } => ("refused", format!("  — {reason}")),
            EntryOutcome::Failed { error } => ("failed ", format!("  — {error}")),
        };
        let _ = writeln!(out, "  {label} {} {target}{detail}", kind_label(entry.kind));
    }

    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "  {} item(s): {} removed, {} already absent, {} refused, {} failed",
        report.entries.len(),
        report.removed(),
        report.absent(),
        report.refused(),
        report.failed()
    );
    if report.failed() > 0 || report.refused() > 0 {
        let _ = writeln!(
            out,
            "  Items reported as refused or failed were not removed and are still present."
        );
    }
    out
}

fn kind_label(kind: PurgeKind) -> &'static str {
    match kind {
        PurgeKind::File => "file",
        PurgeKind::Dir => "dir ",
        PurgeKind::Service => "svc ",
        PurgeKind::Registry => "reg ",
    }
}

// ── CLI surface ──

/// `clotocore purge-exec --plan <file> [--pid <parent>]` — the detached helper
/// of §7. Waits for the process that spawned it to release its files, then
/// executes the plan it was handed and nothing else.
///
/// The plan file is left in place: its lifecycle belongs to the caller that
/// created it.
pub fn run_cli(plan_path: &Path, parent_pid: Option<u32>, json: bool) -> anyhow::Result<()> {
    if let Some(pid) = parent_pid {
        eprintln!("clotocore purge-exec: waiting for PID {pid} to exit...");
        if !crate::platform::wait_for_process_exit(pid, PARENT_EXIT_TIMEOUT) {
            // Removing an installation while it is still running is how the
            // plan's targets end up half-deleted and locked. Nothing is
            // touched in this case.
            bail!(
                "Parent process {pid} did not exit within {}s; nothing was removed",
                PARENT_EXIT_TIMEOUT.as_secs()
            );
        }
    }

    let plan = load_plan(plan_path)?;
    let report = execute(&plan);

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render_text(&report));
    }

    unfinished(&report)?;
    Ok(())
}

/// `clotocore uninstall --execute` — generate a plan, show it, and (after an
/// explicit confirmation) execute it in-process.
///
/// This is the headless counterpart of the dashboard's Danger Zone: same plan
/// generator, same executor, same wording. No detached helper is involved, so
/// it cannot remove a running binary — that is what the `purge-exec` handoff
/// is for.
pub fn run_uninstall(tier: u8, prefix: Option<PathBuf>, assume_yes: bool) -> anyhow::Result<()> {
    let tier = PurgeTier::from_level(tier)
        .ok_or_else(|| anyhow::anyhow!("Invalid scope tier {tier}; expected 1, 2, 3 or 4"))?;
    let plan =
        purge::build_plan(&PlanRequest::new(crate::config::data_dir(), tier).with_prefix(prefix));

    print!("{}", purge::render_text(&plan));

    if plan.entries.is_empty() {
        println!("Nothing to remove at this tier.");
        return Ok(());
    }

    if !assume_yes && !confirm(plan.entries.len())? {
        println!("Cancelled. Nothing was removed.");
        return Ok(());
    }

    let report = execute(&plan);
    print!("{}", render_text(&report));

    unfinished(&report)?;
    Ok(())
}

/// Fail the process when the run did not do what it was asked to.
///
/// `failed` is the obvious case. `refused` matters just as much and is easy to
/// get wrong: a plan this binary cannot execute — a version it does not know,
/// a path that does not survive the floor — produces a report full of refusals
/// and an empty disk operation. Exiting 0 there would tell the caller that
/// spawned the detached helper (§7) that the uninstall succeeded when nothing
/// was removed at all.
fn unfinished(report: &PurgeReport) -> anyhow::Result<()> {
    let (failed, refused) = (report.failed(), report.refused());
    if failed + refused == 0 {
        return Ok(());
    }
    bail!(
        "{} item(s) were not removed: {failed} failed, {refused} refused by the executor's safety \
         floor",
        failed + refused
    );
}

/// Typed confirmation (`yes`, not `y`): the deliberateness gate of §7 in its
/// headless form.
fn confirm(count: usize) -> anyhow::Result<bool> {
    use std::io::Write as _;

    print!("Type 'yes' to remove the {count} item(s) listed above: ");
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(input.trim().eq_ignore_ascii_case("yes"))
}

fn load_plan(path: &Path) -> anyhow::Result<PurgePlan> {
    load_plan_with_limit(path, MAX_PLAN_BYTES)
}

fn load_plan_with_limit(path: &Path, limit: u64) -> anyhow::Result<PurgePlan> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("Cannot open purge plan {}", path.display()))?;
    let declared = file.metadata().map_or(0, |m| m.len());
    if declared > limit {
        bail!(
            "Purge plan {} is {declared} bytes; the limit is {limit}",
            path.display()
        );
    }

    let mut text = String::new();
    // Bound the read as well as the declared size: the length reported by
    // metadata is not a promise about what the stream yields.
    file.take(limit + 1)
        .read_to_string(&mut text)
        .with_context(|| format!("Cannot read purge plan {}", path.display()))?;
    if text.len() as u64 > limit {
        bail!(
            "Purge plan {} exceeds the {limit} byte limit",
            path.display()
        );
    }

    serde_json::from_str(&text)
        .with_context(|| format!("Purge plan {} is not a valid plan", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defender::purge::PurgeSource;

    fn entry(id: &str, kind: PurgeKind, path: Option<&str>) -> PurgeEntry {
        PurgeEntry {
            id: id.to_string(),
            kind,
            path: path.map(ToString::to_string),
            name: None,
            tier: PurgeTier::Everything,
            source: PurgeSource::Receipt,
            size_bytes: None,
            size_truncated: false,
            unreadable: false,
            secret: false,
            covers_secret: false,
        }
    }

    fn plan_of(entries: Vec<PurgeEntry>) -> PurgePlan {
        PurgePlan {
            plan_version: PLAN_VERSION,
            app_version: "test".to_string(),
            generated_at: "2026-07-25T00:00:00Z".to_string(),
            tier: PurgeTier::Everything,
            data_dir: String::new(),
            entries,
            skipped: Vec::new(),
            notes: Vec::new(),
        }
    }

    fn touch(path: &Path, bytes: usize) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, vec![0_u8; bytes]).unwrap();
    }

    fn outcome_of<'a>(report: &'a PurgeReport, id: &str) -> &'a EntryOutcome {
        &report
            .entries
            .iter()
            .find(|e| e.id == id)
            .unwrap_or_else(|| panic!("no report entry for {id}"))
            .outcome
    }

    fn refusal(report: &PurgeReport, id: &str) -> String {
        match outcome_of(report, id) {
            EntryOutcome::Refused { reason } => reason.clone(),
            other => panic!("{id} was not refused: {other:?}"),
        }
    }

    fn as_str(path: &Path) -> String {
        path.to_str().unwrap().to_string()
    }

    // ── Removal ──

    #[test]
    fn files_and_directories_in_the_plan_are_removed() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("cloto_memories.db");
        let dir = root.path().join("mcp-servers");
        touch(&file, 16);
        touch(&dir.join("cscheduler/server.py"), 8);

        let report = execute(&plan_of(vec![
            entry("db", PurgeKind::File, Some(&as_str(&file))),
            entry("mcp_servers_root", PurgeKind::Dir, Some(&as_str(&dir))),
        ]));

        assert_eq!(*outcome_of(&report, "db"), EntryOutcome::Removed);
        assert_eq!(
            *outcome_of(&report, "mcp_servers_root"),
            EntryOutcome::Removed
        );
        assert_eq!(report.removed(), 2);
        assert!(!file.exists(), "the file must be gone");
        assert!(!dir.exists(), "the directory tree must be gone");
    }

    #[test]
    fn a_missing_target_is_absent_not_an_error() {
        // Re-running an interrupted uninstall must converge, and a plan is
        // generated before anything is removed — every entry is a race with
        // the user's own cleanup.
        let root = tempfile::tempdir().unwrap();
        let report = execute(&plan_of(vec![
            entry(
                "db",
                PurgeKind::File,
                Some(&as_str(&root.path().join("gone.db"))),
            ),
            entry(
                "data_dir",
                PurgeKind::Dir,
                Some(&as_str(&root.path().join("gone-dir"))),
            ),
        ]));

        assert_eq!(*outcome_of(&report, "db"), EntryOutcome::Absent);
        assert_eq!(*outcome_of(&report, "data_dir"), EntryOutcome::Absent);
        assert_eq!(report.failed(), 0);
        assert_eq!(report.absent(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_is_unlinked_and_its_target_survives() {
        // The plan measured the link, not the tree behind it. Following it
        // would delete bytes the user was never shown — the plan's own sizing
        // deliberately does not follow links either.
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real-data");
        let payload = real.join("payload.db");
        touch(&payload, 64);
        let link = root.path().join("linked-data");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        // A link whose target is a *file* while the plan says `Dir`. Nothing
        // else in this module removes a target whose type disagrees with the
        // plan, so this case is what makes the symlink branch load-bearing:
        // without it the entry would be refused as a type change, and a link
        // the uninstall promised to remove would be left behind.
        let file_target = root.path().join("elsewhere.db");
        touch(&file_target, 8);
        let link_to_file = root.path().join("linked-file");
        std::os::unix::fs::symlink(&file_target, &link_to_file).unwrap();

        let report = execute(&plan_of(vec![
            entry("data_dir", PurgeKind::Dir, Some(&as_str(&link))),
            entry("webview", PurgeKind::Dir, Some(&as_str(&link_to_file))),
        ]));

        assert_eq!(*outcome_of(&report, "data_dir"), EntryOutcome::Removed);
        assert!(
            !link.exists() && link.symlink_metadata().is_err(),
            "the link itself must be gone"
        );
        assert!(real.is_dir(), "the link target must survive");
        assert!(payload.is_file(), "the target's contents must survive");

        assert_eq!(*outcome_of(&report, "webview"), EntryOutcome::Removed);
        assert!(
            link_to_file.symlink_metadata().is_err(),
            "a link is unlinked whatever the plan called it"
        );
        assert!(file_target.is_file(), "its target must survive untouched");
    }

    // ── Safety floor ──

    #[test]
    fn a_filesystem_root_is_refused() {
        // Asserted against the floor itself, not through `execute`: routing a
        // root through the executor would mean that a regression in
        // `is_filesystem_root` makes *running the test suite* attempt
        // `remove_dir_all("/")`. A guard whose test is destructive when the
        // guard breaks is not a guard worth having.
        for root_path in ["/", r"C:\", r"\\share"] {
            let err = checked_path(Some(root_path))
                .expect_err("a filesystem root must never become a removable path");
            assert!(
                err.contains("filesystem root") || err.contains("not absolute"),
                "{root_path} was refused for the wrong reason: {err}"
            );
        }

        let native_root = if cfg!(windows) { r"C:\" } else { "/" };
        assert!(
            checked_path(Some(native_root))
                .unwrap_err()
                .contains("filesystem root"),
            "this platform's own root must be refused by name, not merely as a bad path"
        );
    }

    #[test]
    fn relative_paths_and_parent_components_are_refused() {
        let root = tempfile::tempdir().unwrap();
        let victim = root.path().join("victim.db");
        touch(&victim, 8);
        std::fs::create_dir_all(root.path().join("sub")).unwrap();
        // A traversal that resolves back onto a real file: the plan file is an
        // input, so `..` must be rejected before it can escape anywhere.
        let traversal = as_str(&root.path().join("sub").join("..").join("victim.db"));

        let report = execute(&plan_of(vec![
            entry("relative", PurgeKind::File, Some("relative/thing.db")),
            entry("traversal", PurgeKind::File, Some(&traversal)),
            entry("empty", PurgeKind::Dir, Some("   ")),
            entry("missing", PurgeKind::File, None),
        ]));

        assert!(refusal(&report, "relative").contains("not absolute"));
        assert!(refusal(&report, "traversal").contains("parent-directory"));
        assert!(refusal(&report, "empty").contains("no path"));
        assert!(refusal(&report, "missing").contains("no path"));
        assert_eq!(report.refused(), 4);
        assert_eq!(report.removed(), 0);
        assert!(victim.is_file(), "nothing may be removed by a refused plan");
    }

    #[test]
    fn a_service_entry_without_a_name_is_refused() {
        // Guards the deregistration branch without invoking it: calling
        // `uninstall_service` in a test would deregister the developer's own
        // installation.
        let mut svc = entry("service", PurgeKind::Service, None);
        svc.name = Some("  ".to_string());
        let report = execute(&plan_of(vec![svc]));
        assert!(refusal(&report, "service").contains("no service name"));
    }

    #[test]
    fn only_this_application_s_uninstall_key_may_be_removed() {
        // `reg delete /f` is recursive and the helper runs elevated, so this
        // predicate is the whole capability boundary for the registry.
        // The four keys the planner actually emits (purge.rs) must pass, or a
        // legitimate uninstall would refuse its own work.
        for hive in ["HKLM", "HKCU"] {
            for name in ["ClotoCore", "cloto-system"] {
                let key =
                    format!(r"{hive}\Software\Microsoft\Windows\CurrentVersion\Uninstall\{name}");
                assert!(is_uninstall_entry_key(&key), "{key} is what purge.rs emits");
            }
        }
        assert!(
            is_uninstall_entry_key(
                r"hklm\software\microsoft\windows\currentversion\uninstall\ClotoCore"
            ),
            "registry paths are case-insensitive"
        );

        for bad in [
            r"HKLM\SOFTWARE",
            r"HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall",
            r"HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall\ClotoCore\Inner",
            r"HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall\",
            r"HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall\*",
            r"HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall\ \",
            r"HKCR\Software\Microsoft\Windows\CurrentVersion\Uninstall\ClotoCore",
            r"HKEY_LOCAL_MACHINE\Software\Microsoft\Windows\CurrentVersion\Uninstall\ClotoCore",
            r"HKLM/Software/Microsoft/Windows/CurrentVersion/Uninstall/ClotoCore",
            r"HKLM\Software\Microsoft\Windows\Uninstall\ClotoCore",
            r"HKLM\Software\..\Software\Microsoft\Windows\CurrentVersion\Uninstall\X",
        ] {
            assert!(
                !is_uninstall_entry_key(bad),
                "{bad} must never be removable"
            );
        }
    }

    #[test]
    fn a_registry_key_outside_the_uninstall_list_is_refused() {
        // Checked before the Windows gate, so the boundary is verified on every
        // platform rather than only where it can be exercised.
        let report = execute(&plan_of(vec![entry(
            "registry_uninstall_hklm",
            PurgeKind::Registry,
            Some(r"HKLM\SOFTWARE"),
        )]));
        assert!(
            refusal(&report, "registry_uninstall_hklm").contains("outside the uninstall list"),
            "an elevated recursive delete of a whole hive must be refused by name"
        );
    }

    #[test]
    fn an_entry_wider_than_the_declared_tier_is_refused() {
        // The tier is printed in the report header, so honouring an entry above
        // it would remove user data under a heading that says "application
        // only".
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("cloto_memories.db");
        touch(&file, 8);
        let mut wide = entry("db", PurgeKind::File, Some(&as_str(&file)));
        wide.tier = PurgeTier::Everything;

        let mut plan = plan_of(vec![wide]);
        plan.tier = PurgeTier::Application;
        let report = execute(&plan);

        let reason = refusal(&report, "db");
        assert!(
            reason.contains("tier 4") && reason.contains("tier 1"),
            "{reason}"
        );
        assert!(
            file.is_file(),
            "a tier-1 uninstall must not remove tier-4 data"
        );
    }

    #[test]
    fn a_plan_from_another_format_version_removes_nothing() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("cloto_memories.db");
        touch(&file, 8);
        let mut plan = plan_of(vec![entry("db", PurgeKind::File, Some(&as_str(&file)))]);
        plan.plan_version = PLAN_VERSION + 99;

        let report = execute(&plan);

        assert!(refusal(&report, "db").contains("plan version mismatch"));
        assert_eq!(report.refused(), 1);
        assert_eq!(report.removed(), 0);
        assert!(file.is_file(), "a plan we cannot read must remove nothing");
        assert_eq!(
            report.plan_version,
            PLAN_VERSION + 99,
            "the report states the version it was handed, not the one we execute"
        );
    }

    #[test]
    fn a_type_change_since_enumeration_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let now_a_file = root.path().join("was-a-dir");
        touch(&now_a_file, 8);
        let now_a_dir = root.path().join("was-a-file");
        std::fs::create_dir_all(now_a_dir.join("child")).unwrap();

        let report = execute(&plan_of(vec![
            entry("data_dir", PurgeKind::Dir, Some(&as_str(&now_a_file))),
            entry("db", PurgeKind::File, Some(&as_str(&now_a_dir))),
        ]));

        assert!(refusal(&report, "data_dir").contains("type changed"));
        assert!(refusal(&report, "db").contains("type changed"));
        assert!(now_a_file.is_file(), "the file must survive");
        assert!(now_a_dir.is_dir(), "the directory must survive");
    }

    // ── Failure handling ──

    #[test]
    fn one_failure_does_not_abandon_the_rest_of_the_plan() {
        // An interior NUL cannot be handed to any OS path API, so the probe
        // fails with something other than NotFound on every platform — a
        // deterministic stand-in for the permission error the real helper
        // hits before elevation.
        let root = tempfile::tempdir().unwrap();
        let before = root.path().join("a-first.db");
        let after = root.path().join("z-last.db");
        touch(&before, 8);
        touch(&after, 8);
        let unusable = format!("{}/na\0me.db", as_str(root.path()));

        let report = execute(&plan_of(vec![
            entry("first", PurgeKind::File, Some(&as_str(&before))),
            entry("broken", PurgeKind::File, Some(&unusable)),
            entry("last", PurgeKind::File, Some(&as_str(&after))),
        ]));

        assert!(
            matches!(outcome_of(&report, "broken"), EntryOutcome::Failed { .. }),
            "expected a failure, got {:?}",
            outcome_of(&report, "broken")
        );
        assert_eq!(*outcome_of(&report, "first"), EntryOutcome::Removed);
        assert_eq!(
            *outcome_of(&report, "last"),
            EntryOutcome::Removed,
            "an entry after the failure must still be processed"
        );
        assert_eq!(report.entries.len(), 3, "every entry gets an outcome");
        assert!(!before.exists() && !after.exists());
    }

    // ── Ordering ──

    #[test]
    fn deregistrations_run_first_and_paths_deepest_first() {
        // The planner's ordering is what makes deregistration work at all
        // (a launchd job cannot be unloaded once its plist is deleted), so the
        // executor must consume the plan through it rather than in file order.
        // A registry entry stands in for the service: it exercises the same
        // ordering rank with no side effect off Windows, and names a key that
        // does not exist on Windows either.
        let root = tempfile::tempdir().unwrap();
        let shallow = root.path().join("shallow.db");
        let deep = root.path().join("a/b/c/deep.db");
        touch(&shallow, 1);
        touch(&deep, 1);

        let report = execute(&plan_of(vec![
            entry("db", PurgeKind::File, Some(&as_str(&shallow))),
            entry("nested", PurgeKind::File, Some(&as_str(&deep))),
            entry(
                "registry_uninstall",
                PurgeKind::Registry,
                Some(r"HKCU\Software\ClotoCore\NoSuchKeyForTests"),
            ),
        ]));

        let order: Vec<&str> = report.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(
            order,
            vec!["registry_uninstall", "nested", "db"],
            "deregistrations first, then the deepest path"
        );
    }

    // ── Rendering and loading ──

    #[test]
    fn rendering_names_every_outcome_and_says_what_survived() {
        let root = tempfile::tempdir().unwrap();
        let removed = root.path().join("removed.db");
        touch(&removed, 4);
        let report = execute(&plan_of(vec![
            entry("db", PurgeKind::File, Some(&as_str(&removed))),
            entry(
                "absent",
                PurgeKind::File,
                Some(&as_str(&root.path().join("nope.db"))),
            ),
            entry("bad", PurgeKind::Dir, Some("relative/dir")),
        ]));

        let text = render_text(&report);
        assert!(text.contains("Scope tier:     4"), "{text}");
        assert!(text.contains("removed file"), "{text}");
        assert!(text.contains("absent"), "{text}");
        assert!(text.contains("refused"), "{text}");
        assert!(
            text.contains("1 removed, 1 already absent, 1 refused, 0 failed"),
            "{text}"
        );
        assert!(
            text.contains("still present"),
            "a report must not read as a complete uninstall when it is not: {text}"
        );
    }

    #[test]
    fn an_empty_plan_reports_nothing_to_remove() {
        let report = execute(&plan_of(Vec::new()));
        assert_eq!(report.entries.len(), 0);
        assert!(render_text(&report).contains("Nothing to remove"));
    }

    #[test]
    fn report_round_trips_through_json() {
        let report = execute(&plan_of(vec![entry("bad", PurgeKind::Dir, Some("rel/x"))]));
        let json = serde_json::to_string(&report).unwrap();
        let back: PurgeReport = serde_json::from_str(&json).unwrap();
        assert_eq!(report, back, "the report is the caller's only account");
    }

    #[test]
    fn a_run_that_removed_nothing_does_not_report_success() {
        // The helper's exit code is the only thing the spawning kernel (§7) can
        // observe. A plan full of refusals removes nothing, so reporting 0 here
        // would read as a completed uninstall.
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("cloto_memories.db");
        touch(&file, 8);

        let mut stale = plan_of(vec![entry("db", PurgeKind::File, Some(&as_str(&file)))]);
        stale.plan_version = PLAN_VERSION + 1;
        let stale_path = root.path().join("stale-plan.json");
        std::fs::write(&stale_path, serde_json::to_vec(&stale).unwrap()).unwrap();

        let err = run_cli(&stale_path, None, false)
            .expect_err("a plan that removed nothing must not exit successfully")
            .to_string();
        assert!(err.contains("1 refused"), "{err}");
        assert!(file.is_file(), "and it must genuinely have removed nothing");

        // The clean case still succeeds, so the check is not simply always-fail.
        let good = plan_of(vec![entry("db", PurgeKind::File, Some(&as_str(&file)))]);
        let good_path = root.path().join("plan.json");
        std::fs::write(&good_path, serde_json::to_vec(&good).unwrap()).unwrap();
        run_cli(&good_path, None, false).expect("a plan that fully applied must exit 0");
        assert!(!file.exists());
    }

    #[test]
    fn loading_refuses_an_oversized_plan_and_rejects_junk() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("plan.json");
        std::fs::write(&path, serde_json::to_vec(&plan_of(Vec::new())).unwrap()).unwrap();

        assert!(load_plan(&path).is_ok(), "a real plan must load");
        // The size is refused before the file is read at all; the bound on the
        // read itself is a second line of defence for a stream that does not
        // match its metadata, so the wording pins which one fired.
        let err = load_plan_with_limit(&path, 4).unwrap_err().to_string();
        assert!(err.contains("the limit is 4"), "{err}");

        std::fs::write(&path, b"{ not a plan").unwrap();
        assert!(load_plan(&path).is_err(), "junk must not parse into a plan");

        assert!(
            load_plan(&root.path().join("missing.json")).is_err(),
            "a missing plan is an error, not an empty run"
        );
    }
}
