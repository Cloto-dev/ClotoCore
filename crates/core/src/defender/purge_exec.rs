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
//!
//! # Why the lexical floor is not the whole boundary
//!
//! Everything above is a check on the *shape* of a path. Shape is not
//! ownership: `/etc/passwd` is absolute, is not a root, and carries no `..`.
//! The helper runs elevated on Windows (§7), and the plan reaches it through a
//! file that a same-user process could rewrite between the moment the kernel
//! writes it and the moment the elevated helper reads it — so if the plan's
//! contents were the only thing deciding what gets deleted, plan-file integrity
//! would be the entire security boundary.
//!
//! [`PurgeRoots`] closes that: the caller states, out of band from the plan,
//! which directory trees this run may touch, and an entry outside every one of
//! them is refused no matter how well-formed it looks. The kernel passes the
//! roots as command-line arguments — which the process that rewrites a file on
//! disk cannot reach — and a helper invoked without any derives them from its
//! own environment rather than from the plan it was handed. An edited plan can
//! then still name only things inside ClotoCore's own footprint.

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};

use crate::defender::footprint;
use crate::defender::purge::{
    self, is_filesystem_root, order_for_removal, tier_label, PlanRequest, ProbeRoots, PurgeEntry,
    PurgeKind, PurgePlan, PurgeTier, PLAN_VERSION,
};

/// A plan is a list of paths, not a payload. Anything larger than this is not
/// a plan this binary wrote, and it is read from disk before any of it is
/// trusted — so the bound comes first.
const MAX_PLAN_BYTES: u64 = 8 * 1024 * 1024;

/// How long the detached helper waits for the kernel that spawned it to exit
/// (§7 choreography). Matches the updater's `execute_swap` handoff window.
const PARENT_EXIT_TIMEOUT: Duration = Duration::from_secs(30);

/// Written next to the plan after a run. A detached helper's stdout goes
/// nowhere — the process that spawned it has exited by then — so without this
/// the only thing that survives the run is a one-bit exit status.
pub const REPORT_FILE_SUFFIX: &str = ".report.json";

/// File name a saved plan gets, on both paths. The detached handoff
/// (`uninstall::stage`) and the in-process run write the same artifact, and
/// resuming either is the same `purge-exec --plan <file>`.
pub(crate) const PLAN_FILE: &str = "purge-plan.json";

// ── Containment ──

/// The directory trees an executed plan is allowed to touch.
///
/// This is the half of the boundary the plan file cannot state about itself
/// (see the module docs). Roots are compared lexically against paths that have
/// already passed the floor in [`checked_path`] — absolute, no `..` — so no
/// filesystem access is needed to decide, and a symlink cannot widen the set:
/// the executor unlinks links rather than following them.
#[derive(Debug, Clone, Default)]
pub struct PurgeRoots {
    roots: Vec<PathBuf>,
}

impl PurgeRoots {
    /// Roots stated by the caller. Relative entries are dropped: a relative
    /// root would match nothing, and silently keeping one invites the reading
    /// that it widened the set.
    #[must_use]
    pub fn from_paths(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut roots: Vec<PathBuf> = paths
            .into_iter()
            .filter(|p| p.is_absolute() && !is_filesystem_root(p))
            .collect();
        roots.sort();
        roots.dedup();
        Self { roots }
    }

    /// What this installation could legitimately own, derived from the running
    /// binary's own environment — never from the plan, which is the input under
    /// suspicion.
    #[must_use]
    pub fn from_env(data_dir: &Path, prefix: Option<&Path>) -> Self {
        Self::from_probe(data_dir, prefix, &ProbeRoots::from_env())
    }

    /// The same derivation from a stated machine layout rather than this one.
    ///
    /// Exists so the set can be checked against what the *planner* emits from
    /// the same layout (`purge::tests::every_path_the_planner_emits_…`). The two
    /// notions of "belongs to this installation" are independent by design — the
    /// plan is the input under suspicion — but they must not contradict each
    /// other, or the uninstall refuses paths it offered to remove.
    ///
    /// Each platform location is admitted as narrowly as the artifact allows:
    /// the Start Menu directory rather than all of `%ProgramData%`, the public
    /// desktop rather than the whole public profile, `applications` and
    /// `icons/hicolor` rather than all of `/usr/share`.
    #[must_use]
    pub fn from_probe(data_dir: &Path, prefix: Option<&Path>, probe: &ProbeRoots) -> Self {
        let mut paths = vec![data_dir.to_path_buf()];
        paths.extend(prefix.map(Path::to_path_buf));
        paths.extend(probe.home.clone());
        paths.extend(probe.platform_data.clone());
        paths.extend(probe.platform_cache.clone());
        paths.extend(probe.platform_local.clone());
        paths.extend(probe.exe_dir.clone());
        if cfg!(target_os = "macos") {
            paths.push(PathBuf::from("/Applications"));
        }
        if cfg!(target_os = "windows") {
            paths.extend(
                probe
                    .program_data
                    .as_ref()
                    .map(|d| d.join(crate::defender::purge::START_MENU_REL)),
            );
            paths.extend(probe.public.as_ref().map(|d| d.join("Desktop")));
            paths.extend(probe.desktop.clone());
        }
        if cfg!(target_os = "linux") {
            // The systemd unit has been in every Linux plan since the plan
            // existed, and nothing could remove it: no root covered /etc.
            paths.push(PathBuf::from("/etc/systemd/system"));
            if let Some(share) = &probe.system_share {
                paths.push(share.join("applications"));
                paths.push(share.join("icons").join("hicolor"));
            }
        }
        Self::from_paths(paths)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    #[must_use]
    pub fn paths(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Is `path` inside a declared root? The executor's own admission test,
    /// exposed so the planner's output can be checked against it.
    #[must_use]
    pub fn covers(&self, path: &Path) -> bool {
        self.contains(path)
    }

    fn contains(&self, path: &Path) -> bool {
        self.roots.iter().any(|root| is_within(root, path))
    }
}

/// Is `path` `root` itself or something under it?
///
/// Component-wise, so `/opt/cloto-evil` is not inside `/opt/cloto`. Windows
/// paths are case-insensitive, and the plan is written by one process and read
/// by another that may have resolved the same directory with different casing;
/// comparing case-sensitively there would refuse an installation's own files.
fn is_within(root: &Path, path: &Path) -> bool {
    if cfg!(windows) {
        let fold = |p: &Path| -> Vec<std::ffi::OsString> {
            p.components()
                .map(|c| c.as_os_str().to_ascii_lowercase())
                .collect()
        };
        let (root, path) = (fold(root), fold(path));
        path.len() >= root.len() && path[..root.len()] == root[..]
    } else {
        path.starts_with(root)
    }
}

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

/// Execute `plan`, removing only what it lists *and* only what `roots` allows.
///
/// Never panics and never returns early: every entry gets an outcome, so the
/// report is a complete account of the run even when parts of it failed.
#[must_use]
pub fn execute(plan: &PurgePlan, roots: &PurgeRoots) -> PurgeReport {
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

    // Deregistrations first, then deepest paths first, and whatever holds the
    // install receipt last (the planner's own ordering rule — a launchd job
    // cannot be unloaded once its plist is gone, and a receipt removed early
    // takes the next run's enumeration with it). Consuming the plan through
    // the same function is what keeps the executor from undoing that order.
    //
    // `data_dir` is a plan field, and a plan cannot widen what it may touch:
    // this decides the *order* only. What may be touched is stated by `roots`,
    // out of band from the file.
    let receipt_path =
        non_empty(Some(plan.data_dir.as_str())).map(|dir| footprint::receipt_path(Path::new(dir)));
    for entry in order_for_removal(plan.entries.clone(), receipt_path.as_deref()) {
        // The tier the plan declares is a floor like any other, not a caption.
        // The planner filters by it (`purge.rs`), and the report prints it — so
        // an entry wider than the declared scope would be removed while the
        // header said "application only".
        let outcome = if plan.tier.includes(entry.tier) {
            execute_entry(&entry, roots)
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

fn execute_entry(entry: &PurgeEntry, roots: &PurgeRoots) -> EntryOutcome {
    match entry.kind {
        PurgeKind::File | PurgeKind::Dir => match checked_path(entry.path.as_deref()) {
            Err(reason) => refuse(reason),
            Ok(path) => match contained(&path, roots) {
                Err(reason) => refuse(reason),
                Ok(()) => remove_path(&path, entry.kind),
            },
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

/// Refuse a path that no declared root covers.
///
/// An empty root set refuses everything rather than allowing everything: the
/// set is derived from the environment when the caller states nothing, so an
/// empty one means the derivation failed, and "we could not work out what this
/// installation owns" must not read as "delete anything".
fn contained(path: &Path, roots: &PurgeRoots) -> Result<(), String> {
    if roots.is_empty() {
        return Err(format!(
            "no purge roots were established, so nothing can be shown to belong to this \
             installation: {}",
            path.display()
        ));
    }
    if roots.contains(path) {
        return Ok(());
    }
    Err(format!(
        "path is outside this installation's footprint: {} (allowed: {})",
        path.display(),
        roots
            .paths()
            .iter()
            .map(|r| r.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.trim().is_empty())
}

/// Would executing `plan` need more privilege than the current user has?
///
/// Used by the kernel to decide whether to ask for elevation *before* it exits
/// (§7). Deliberately a question about the plan rather than a permission probe:
/// probing races with the very state it measures, and a wrong "no" surfaces as
/// a helper that fails after the app is already gone, with no way to ask.
///
/// `home` is the user's own tree; anything under it, or under no path at all
/// (a service or per-user registry entry), is theirs to remove.
#[must_use]
pub fn requires_elevation(plan: &PurgePlan, home: Option<&Path>) -> bool {
    plan.entries.iter().any(|entry| match entry.kind {
        // Deregistering a machine-wide service is an administrative act
        // wherever its files happen to live.
        PurgeKind::Service => true,
        PurgeKind::Registry => entry
            .path
            .as_deref()
            .is_some_and(|key| key.to_ascii_uppercase().starts_with("HKLM")),
        PurgeKind::File | PurgeKind::Dir => match (&entry.path, home) {
            (Some(path), Some(home)) => !is_within(home, Path::new(path)),
            // No home to compare against: assume the wider answer, since the
            // cost of a needless prompt is a prompt, and the cost of a missing
            // one is an uninstall that silently leaves the install dir behind.
            (Some(_), None) => true,
            (None, _) => false,
        },
    })
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
///
/// `roots` is the containment set (see the module docs). An empty one is not
/// "no restriction" — it means the caller stated nothing, so the helper derives
/// the set from its own environment.
pub fn run_cli(
    plan_path: &Path,
    parent_pid: Option<u32>,
    json: bool,
    roots: PurgeRoots,
) -> anyhow::Result<()> {
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
    let roots = if roots.is_empty() {
        PurgeRoots::from_env(&crate::config::data_dir(), None)
    } else {
        roots
    };
    let report = execute(&plan, &roots);

    // Before rendering: the account has to survive even if this process is
    // killed while printing, and stdout is not where a detached helper's caller
    // will look.
    let written = write_report_beside(plan_path, &report);

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render_text(&report));
    }
    match written {
        Ok(path) => println!("Report written to {}", path.display()),
        Err(e) => println!("Report could not be written next to the plan: {e:#}"),
    }

    unfinished(&report)?;
    Ok(())
}

/// Persist the report beside the plan that produced it.
///
/// Same directory as the plan on purpose: that directory is the one the caller
/// chose and knows the path of, and on the detached path (§7) it is the only
/// thing left that both processes agreed on.
fn write_report_beside(plan_path: &Path, report: &PurgeReport) -> anyhow::Result<PathBuf> {
    let mut name = plan_path.file_name().unwrap_or_default().to_os_string();
    name.push(REPORT_FILE_SUFFIX);
    let path = plan_path.with_file_name(name);
    let body = serde_json::to_vec_pretty(report).context("Cannot serialise the purge report")?;
    std::fs::write(&path, body)
        .with_context(|| format!("Cannot write purge report {}", path.display()))?;
    Ok(path)
}

/// `clotocore uninstall --execute` — generate a plan, show it, and (after an
/// explicit confirmation) execute it in-process.
///
/// This is the headless counterpart of the dashboard's Danger Zone: same plan
/// generator, same executor, same wording. No detached helper is involved, so
/// it cannot remove a running binary — that is what the `purge-exec` handoff
/// is for.
pub fn run_uninstall(
    tier: u8,
    prefix: Option<PathBuf>,
    assume_yes: bool,
    json: bool,
) -> anyhow::Result<()> {
    let tier = PurgeTier::from_level(tier)
        .ok_or_else(|| anyhow::anyhow!("Invalid scope tier {tier}; expected 1, 2, 3 or 4"))?;
    let data_dir = crate::config::data_dir();

    // A live kernel keeps writing to the tree this is about to remove: on Unix
    // the deletion succeeds against open files, the running process keeps
    // writing to unlinked inodes, and it recreates the receipt and data
    // directory as it shuts down. The uninstall reports success and the
    // installation is still there. In-process removal is only correct when
    // nothing is running; the detached handoff (§7) is what exists for the
    // other case.
    if let Some(pid) = crate::defender::runlock::live_holder(&data_dir) {
        bail!(
            "ClotoCore is running (PID {pid}). Stop it first, or uninstall from the app \
             (Settings → Health → Danger Zone), which hands the removal to a detached helper. \
             Nothing was removed."
        );
    }

    let plan =
        purge::build_plan(&PlanRequest::new(data_dir.clone(), tier).with_prefix(prefix.clone()));

    print!("{}", purge::render_text(&plan));

    if plan.entries.is_empty() {
        println!("Nothing to remove at this tier.");
        return Ok(());
    }

    if !assume_yes && !confirm(plan.entries.len())? {
        println!("Cancelled. Nothing was removed.");
        return Ok(());
    }

    let roots = PurgeRoots::from_env(&data_dir, prefix.as_deref());

    // The plan is saved before anything is removed, into a directory outside
    // the tree this is about to delete. The detached path has always done this
    // (`uninstall::stage`); doing it here too is what makes a partial failure
    // recoverable, because the saved plan still names everything the receipt
    // knew about — and the receipt is one of the things a tier-4 run removes.
    let staging = crate::defender::uninstall::create_staging_dir()?;
    let plan_path = stage_plan(&staging, &plan)?;
    println!("Plan written to {}", plan_path.display());

    let report = execute(&plan, &roots);
    let written = write_report_beside(&plan_path, &report);

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render_text(&report));
    }
    match written {
        Ok(path) => println!("Report written to {}", path.display()),
        Err(e) => println!("Report could not be written next to the plan: {e:#}"),
    }

    unfinished_in_process(&report, &plan_path)?;
    Ok(())
}

/// Save `plan` into `staging` and return where it landed.
///
/// Written before the removal starts, not after: a run that dies halfway has
/// to leave behind the one file that can finish it.
fn stage_plan(staging: &Path, plan: &PurgePlan) -> anyhow::Result<PathBuf> {
    let path = staging.join(PLAN_FILE);
    let body = serde_json::to_vec_pretty(plan).context("Cannot serialise the purge plan")?;
    std::fs::write(&path, body)
        .with_context(|| format!("Cannot write the purge plan to {}", path.display()))?;
    Ok(path)
}

/// Fail an in-process run that did not finish, and say how to finish it.
///
/// `unfinished` states what happened; this states what to do about it, and the
/// distinction matters because the obvious remedy is the wrong one.
/// `uninstall --execute` rebuilds its plan from the install receipt, and the
/// receipt is one of the things a wide run removes — so a second `--execute`
/// can enumerate less than the first did, and quietly leave behind whatever
/// only the receipt knew about. The saved plan still names all of it.
fn unfinished_in_process(report: &PurgeReport, plan_path: &Path) -> anyhow::Result<()> {
    if let Err(e) = unfinished(report) {
        bail!("{e:#}\n\n{}", resume_hint(plan_path));
    }
    Ok(())
}

fn resume_hint(plan_path: &Path) -> String {
    format!(
        "Fix the cause, then re-run:  clotocore purge-exec --plan {}\nThat plan still names \
         everything the install receipt knew about. Running `clotocore uninstall --execute` again \
         rebuilds the plan from the receipt instead, which this run may already have removed, so \
         it can no longer name those items.",
        plan_path.display()
    )
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

    /// Containment stated the way a real caller states it: from outside the
    /// plan. Every test therefore also asserts, implicitly, that a legitimate
    /// plan is not refused by its own boundary.
    fn only(root: &Path) -> PurgeRoots {
        PurgeRoots::from_paths([root.to_path_buf()])
    }

    fn execute_in(root: &Path, plan: &PurgePlan) -> PurgeReport {
        execute(plan, &only(root))
    }

    // ── Removal ──

    #[test]
    fn files_and_directories_in_the_plan_are_removed() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("cloto_memories.db");
        let dir = root.path().join("mcp-servers");
        touch(&file, 16);
        touch(&dir.join("cscheduler/server.py"), 8);

        let report = execute_in(
            root.path(),
            &plan_of(vec![
                entry("db", PurgeKind::File, Some(&as_str(&file))),
                entry("mcp_servers_root", PurgeKind::Dir, Some(&as_str(&dir))),
            ]),
        );

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
        let report = execute_in(
            root.path(),
            &plan_of(vec![
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
            ]),
        );

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

        let report = execute_in(
            root.path(),
            &plan_of(vec![
                entry("data_dir", PurgeKind::Dir, Some(&as_str(&link))),
                entry("webview", PurgeKind::Dir, Some(&as_str(&link_to_file))),
            ]),
        );

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

        let report = execute_in(
            root.path(),
            &plan_of(vec![
                entry("relative", PurgeKind::File, Some("relative/thing.db")),
                entry("traversal", PurgeKind::File, Some(&traversal)),
                entry("empty", PurgeKind::Dir, Some("   ")),
                entry("missing", PurgeKind::File, None),
            ]),
        );

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
        let root = tempfile::tempdir().unwrap();
        let mut svc = entry("service", PurgeKind::Service, None);
        svc.name = Some("  ".to_string());
        let report = execute_in(root.path(), &plan_of(vec![svc]));
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
        let root = tempfile::tempdir().unwrap();
        let report = execute_in(
            root.path(),
            &plan_of(vec![entry(
                "registry_uninstall_hklm",
                PurgeKind::Registry,
                Some(r"HKLM\SOFTWARE"),
            )]),
        );
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
        let report = execute_in(root.path(), &plan);

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

        let report = execute_in(root.path(), &plan);

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

        let report = execute_in(
            root.path(),
            &plan_of(vec![
                entry("data_dir", PurgeKind::Dir, Some(&as_str(&now_a_file))),
                entry("db", PurgeKind::File, Some(&as_str(&now_a_dir))),
            ]),
        );

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

        let report = execute_in(
            root.path(),
            &plan_of(vec![
                entry("first", PurgeKind::File, Some(&as_str(&before))),
                entry("broken", PurgeKind::File, Some(&unusable)),
                entry("last", PurgeKind::File, Some(&as_str(&after))),
            ]),
        );

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

        let report = execute_in(
            root.path(),
            &plan_of(vec![
                entry("db", PurgeKind::File, Some(&as_str(&shallow))),
                entry("nested", PurgeKind::File, Some(&as_str(&deep))),
                entry(
                    "registry_uninstall",
                    PurgeKind::Registry,
                    Some(r"HKCU\Software\ClotoCore\NoSuchKeyForTests"),
                ),
            ]),
        );

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
        let report = execute_in(
            root.path(),
            &plan_of(vec![
                entry("db", PurgeKind::File, Some(&as_str(&removed))),
                entry(
                    "absent",
                    PurgeKind::File,
                    Some(&as_str(&root.path().join("nope.db"))),
                ),
                entry("bad", PurgeKind::Dir, Some("relative/dir")),
            ]),
        );

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
        let root = tempfile::tempdir().unwrap();
        let report = execute_in(root.path(), &plan_of(Vec::new()));
        assert_eq!(report.entries.len(), 0);
        assert!(render_text(&report).contains("Nothing to remove"));
    }

    #[test]
    fn report_round_trips_through_json() {
        let root = tempfile::tempdir().unwrap();
        let report = execute_in(
            root.path(),
            &plan_of(vec![entry("bad", PurgeKind::Dir, Some("rel/x"))]),
        );
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

        let err = run_cli(&stale_path, None, false, only(root.path()))
            .expect_err("a plan that removed nothing must not exit successfully")
            .to_string();
        assert!(err.contains("1 refused"), "{err}");
        assert!(file.is_file(), "and it must genuinely have removed nothing");

        // The clean case still succeeds, so the check is not simply always-fail.
        let good = plan_of(vec![entry("db", PurgeKind::File, Some(&as_str(&file)))]);
        let good_path = root.path().join("plan.json");
        std::fs::write(&good_path, serde_json::to_vec(&good).unwrap()).unwrap();
        run_cli(&good_path, None, false, only(root.path()))
            .expect("a plan that fully applied must exit 0");
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

    // ── Containment ──

    #[test]
    fn a_path_outside_every_root_is_refused() {
        // The scenario the roots exist for: the plan file was rewritten between
        // being written and being read, and the helper runs elevated. Every
        // lexical check passes — absolute, not a root, no `..` — and the entry
        // must still be refused.
        let owned = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let victim = elsewhere.path().join("passwd");
        touch(&victim, 32);

        let report = execute_in(
            owned.path(),
            &plan_of(vec![entry("db", PurgeKind::File, Some(&as_str(&victim)))]),
        );

        assert!(
            refusal(&report, "db").contains("outside this installation's footprint"),
            "{}",
            refusal(&report, "db")
        );
        assert!(
            victim.is_file(),
            "an edited plan must not be able to reach outside the declared roots"
        );
    }

    #[test]
    fn an_empty_root_set_removes_nothing() {
        // "We could not work out what this installation owns" must not read as
        // "delete anything" — the failure has to land on the safe side.
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("cloto_memories.db");
        touch(&file, 8);

        let report = execute(
            &plan_of(vec![entry("db", PurgeKind::File, Some(&as_str(&file)))]),
            &PurgeRoots::default(),
        );

        assert!(refusal(&report, "db").contains("no purge roots"));
        assert!(
            file.is_file(),
            "no roots means no removals, not all of them"
        );
    }

    #[test]
    fn a_sibling_of_a_root_is_not_inside_it() {
        // String-prefix containment would treat `/opt/cloto-evil` as living
        // inside `/opt/cloto`. Both directories exist here, so the assertion
        // fails loudly rather than passing because the path was absent.
        let base = tempfile::tempdir().unwrap();
        let owned = base.path().join("cloto");
        let sibling = base.path().join("cloto-evil");
        let inside = owned.join("data.db");
        let outside = sibling.join("data.db");
        touch(&inside, 8);
        touch(&outside, 8);

        let report = execute(
            &plan_of(vec![
                entry("inside", PurgeKind::File, Some(&as_str(&inside))),
                entry("outside", PurgeKind::File, Some(&as_str(&outside))),
            ]),
            &only(&owned),
        );

        assert_eq!(*outcome_of(&report, "inside"), EntryOutcome::Removed);
        assert!(refusal(&report, "outside").contains("outside this installation's footprint"));
        assert!(outside.is_file(), "a sibling directory is a different tree");
    }

    #[test]
    fn a_root_that_could_not_narrow_anything_is_dropped() {
        // A relative root matches nothing and a filesystem root matches
        // everything. Keeping either would misrepresent the set: the first
        // reads as a restriction that is not there, the second as a
        // restriction that does not restrict.
        let roots = PurgeRoots::from_paths([
            PathBuf::from("relative/dir"),
            PathBuf::from("/"),
            PathBuf::from(r"C:\"),
        ]);
        assert!(
            roots.is_empty(),
            "got {:?} — none of these narrows anything",
            roots.paths()
        );
    }

    #[test]
    fn containment_is_stated_by_the_caller_not_by_the_plan() {
        // A plan cannot widen itself: `data_dir` is a plan field, and it is not
        // consulted when deciding what may be touched.
        let owned = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let victim = elsewhere.path().join("payload.db");
        touch(&victim, 8);

        let mut plan = plan_of(vec![entry("db", PurgeKind::File, Some(&as_str(&victim)))]);
        plan.data_dir = as_str(elsewhere.path());

        let report = execute(&plan, &only(owned.path()));
        assert!(refusal(&report, "db").contains("outside this installation's footprint"));
        assert!(victim.is_file());
    }

    #[test]
    fn a_plan_rewritten_on_disk_cannot_reach_past_the_roots() {
        // End to end through the helper's own entry point, because that is
        // where the threat lives: the kernel writes a plan, something rewrites
        // the file, and the helper — elevated, on Windows — reads what is there
        // now. The roots arrive as arguments, so they survive the rewrite.
        let owned = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let mine = owned.path().join("cloto_memories.db");
        let theirs = elsewhere.path().join("someone-elses.db");
        touch(&mine, 8);
        touch(&theirs, 8);

        let plan_path = owned.path().join("purge-plan.json");
        std::fs::write(
            &plan_path,
            serde_json::to_vec(&plan_of(vec![entry(
                "db",
                PurgeKind::File,
                Some(&as_str(&mine)),
            )]))
            .unwrap(),
        )
        .unwrap();

        // The rewrite: same shape, different target.
        std::fs::write(
            &plan_path,
            serde_json::to_vec(&plan_of(vec![entry(
                "db",
                PurgeKind::File,
                Some(&as_str(&theirs)),
            )]))
            .unwrap(),
        )
        .unwrap();

        let err = run_cli(&plan_path, None, false, only(owned.path()))
            .expect_err("a run that removed nothing must not exit 0");

        assert!(err.to_string().contains("1 refused"), "{err}");
        assert!(
            theirs.is_file(),
            "the rewritten target must survive: the plan file is not the whole boundary"
        );
        assert!(
            mine.is_file(),
            "and the original target is not removed either — the plan no longer names it"
        );
    }

    // ── Elevation ──

    #[test]
    fn elevation_is_asked_for_only_when_the_plan_reaches_outside_the_user() {
        let home = PathBuf::from(if cfg!(windows) {
            r"C:\Users\someone"
        } else {
            "/home/someone"
        });
        let mine = as_str(&home.join(".local/share/ClotoCore"));
        let theirs = if cfg!(windows) {
            r"C:\Program Files\ClotoCore"
        } else {
            "/opt/cloto"
        };

        let own_tree = plan_of(vec![entry("data_dir", PurgeKind::Dir, Some(&mine))]);
        assert!(
            !requires_elevation(&own_tree, Some(&home)),
            "removing one's own files must not raise a prompt"
        );

        let install_dir = plan_of(vec![entry("prefix", PurgeKind::Dir, Some(theirs))]);
        assert!(requires_elevation(&install_dir, Some(&home)));

        let mut service = entry("service", PurgeKind::Service, None);
        service.name = Some("Cloto".to_string());
        assert!(
            requires_elevation(&plan_of(vec![service]), Some(&home)),
            "deregistering a machine-wide service is an administrative act"
        );

        let hklm = plan_of(vec![entry(
            "registry_uninstall_hklm",
            PurgeKind::Registry,
            Some(r"HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall\ClotoCore"),
        )]);
        assert!(requires_elevation(&hklm, Some(&home)));

        let hkcu = plan_of(vec![entry(
            "registry_uninstall_hkcu",
            PurgeKind::Registry,
            Some(r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\ClotoCore"),
        )]);
        assert!(
            !requires_elevation(&hkcu, Some(&home)),
            "a per-user key is the user's own"
        );

        assert!(
            requires_elevation(&own_tree, None),
            "with no home to compare against, assume the prompt is needed"
        );
        assert!(
            !requires_elevation(&plan_of(Vec::new()), None),
            "an empty plan asks for nothing"
        );
    }

    // ── Report persistence ──

    #[test]
    fn the_run_is_recorded_next_to_the_plan() {
        // A detached helper's stdout goes nowhere: the process that spawned it
        // has exited. Without this file the only surviving account of what an
        // uninstall did is its exit status.
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("cloto_memories.db");
        touch(&file, 8);
        let plan_path = root.path().join("plan.json");
        std::fs::write(
            &plan_path,
            serde_json::to_vec(&plan_of(vec![entry(
                "db",
                PurgeKind::File,
                Some(&as_str(&file)),
            )]))
            .unwrap(),
        )
        .unwrap();

        run_cli(&plan_path, None, false, only(root.path())).expect("the plan applies cleanly");

        let report_path = root.path().join(format!("plan.json{REPORT_FILE_SUFFIX}"));
        let written: PurgeReport =
            serde_json::from_slice(&std::fs::read(&report_path).expect("a report must be written"))
                .expect("and it must be the report type, not prose");
        assert_eq!(written.removed(), 1);
        assert_eq!(written.entries[0].id, "db");
    }

    #[test]
    fn an_in_process_run_saves_its_plan_and_its_report() {
        // `uninstall --execute` used to print and forget. A partial failure
        // then lost both the plan and the account of what happened — and the
        // receipt a second run would rebuild the plan from is one of the
        // things a wide run removes.
        let staging = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("cloto_memories.db");
        touch(&file, 8);
        let plan = plan_of(vec![entry("db", PurgeKind::File, Some(&as_str(&file)))]);

        let plan_path = stage_plan(staging.path(), &plan).expect("the plan must be saved");
        assert!(
            !plan_path.starts_with(root.path()),
            "a plan stored inside the tree being removed is gone when it is needed"
        );

        let report = execute_in(root.path(), &plan);
        let report_path = write_report_beside(&plan_path, &report).expect("and so must the report");

        let saved: PurgePlan = serde_json::from_slice(&std::fs::read(&plan_path).unwrap())
            .expect("the saved plan is what `purge-exec --plan` reads back");
        assert_eq!(saved, plan);
        let saved: PurgeReport = serde_json::from_slice(&std::fs::read(&report_path).unwrap())
            .expect("the report is JSON, not prose");
        assert_eq!(saved, report);
        assert_eq!(saved.removed(), 1);
    }

    #[test]
    fn an_unfinished_in_process_run_points_at_the_saved_plan() {
        // Telling the user to re-run `uninstall --execute` would be the wrong
        // advice: that path rebuilds its plan from a receipt this run may have
        // removed. Only the saved plan still names everything.
        let root = tempfile::tempdir().unwrap();
        let plan_path = root.path().join(PLAN_FILE);

        let clean = execute_in(root.path(), &plan_of(Vec::new()));
        assert!(
            unfinished_in_process(&clean, &plan_path).is_ok(),
            "a run with nothing left over must still succeed"
        );

        let stuck = execute_in(
            root.path(),
            &plan_of(vec![entry("db", PurgeKind::File, Some("relative/x.db"))]),
        );
        let err = unfinished_in_process(&stuck, &plan_path)
            .expect_err("a run that left items behind must not exit 0")
            .to_string();
        assert!(err.contains("1 refused"), "{err}");
        assert!(err.contains("purge-exec --plan"), "{err}");
        assert!(err.contains(&plan_path.display().to_string()), "{err}");
        assert!(
            err.contains("receipt"),
            "the instruction has to say why the obvious remedy is the wrong one: {err}"
        );
    }

    #[test]
    fn the_executor_removes_the_receipt_bearing_directory_last() {
        // The executor re-orders the plan it is handed, so it has to honour
        // the planner's rule rather than fall back to deepest-first — the
        // container here is deeper than the bundle, so it would go first.
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("opt/cloto/data");
        let bundle = root.path().join("Apps/ClotoCore.app");
        touch(&data.join("cloto_memories.db"), 8);
        touch(&bundle.join("Contents/Info.plist"), 8);

        let mut plan = plan_of(vec![
            entry("data_dir", PurgeKind::Dir, Some(&as_str(&data))),
            entry("app_bundle", PurgeKind::Dir, Some(&as_str(&bundle))),
        ]);
        plan.data_dir = as_str(&data);

        let report = execute_in(root.path(), &plan);
        let order: Vec<&str> = report.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(
            order,
            vec!["app_bundle", "data_dir"],
            "the directory holding the install receipt is removed last"
        );
        assert_eq!(report.removed(), 2, "and both are still removed");
    }

    #[test]
    fn a_failed_run_still_leaves_an_account() {
        // The failing case is the one worth reading afterwards, so the report
        // is written before the exit status is decided.
        let root = tempfile::tempdir().unwrap();
        let plan_path = root.path().join("plan.json");
        let mut stale = plan_of(vec![entry("db", PurgeKind::File, Some("relative/x.db"))]);
        stale.plan_version = PLAN_VERSION + 7;
        std::fs::write(&plan_path, serde_json::to_vec(&stale).unwrap()).unwrap();

        run_cli(&plan_path, None, true, only(root.path()))
            .expect_err("a plan that removed nothing must not exit 0");

        let report_path = root.path().join(format!("plan.json{REPORT_FILE_SUFFIX}"));
        let written: PurgeReport =
            serde_json::from_slice(&std::fs::read(&report_path).expect("a report must be written"))
                .unwrap();
        assert_eq!(written.refused(), 1);
        assert_eq!(
            written.plan_version,
            PLAN_VERSION + 7,
            "the record states the version it was handed"
        );
    }
}
