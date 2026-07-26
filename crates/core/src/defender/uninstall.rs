//! The uninstall flow — what makes the purge executor reachable
//! (`DEFENDER_DESIGN.md` §7, "Self-deletion choreography").
//!
//! A running installation cannot delete itself: the binary is locked on
//! Windows, the databases are open, and on Unix the deletions would appear to
//! succeed while the live process kept writing to unlinked inodes and
//! recreated its data directory on the way out. So the removal is handed to a
//! separate process:
//!
//! ```text
//! stage()   plan → staging dir → copy of this binary
//! launch()  spawn the copy: purge-exec --plan <file> --pid <us> --root <r>…
//! (caller)  drain MCP servers → close the pool → exit
//! helper    wait for our pid to disappear → execute the plan → write a report
//! ```
//!
//! Two things about this module are load-bearing beyond the mechanics.
//!
//! **The roots travel on the command line.** The plan is a file, and a file can
//! be rewritten between the moment we write it and the moment the helper (which
//! is elevated on Windows) reads it. The containment set is therefore passed as
//! arguments, which a process that can only rewrite files cannot reach. See the
//! `purge_exec` module docs.
//!
//! **Elevation is asked for before we exit, not after.** §7 sketched the
//! elevation as happening inside the helper. Doing it there means a UAC prompt
//! appears after the app has vanished — with nothing on screen to explain it,
//! and no way to report a refusal to anyone. Asking while the kernel is still
//! up costs nothing (the helper blocks on our pid regardless) and turns a
//! declined prompt into an ordinary error response with the app still running.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context};

use crate::defender::purge::{self, PlanRequest, PurgePlan, PurgeTier};
use crate::defender::purge_exec::{self, PurgeRoots};

/// How long the user is given to answer the elevation prompt. Long enough to
/// find the dialog behind another window; short enough that a kernel does not
/// hang on it forever.
#[cfg(windows)]
const ELEVATION_PROMPT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// A staged uninstall: everything written to disk, nothing started yet.
#[derive(Debug)]
pub struct StagedUninstall {
    pub plan: PurgePlan,
    /// Directory holding the plan and the copied helper binary. Left behind
    /// deliberately — the report is written into it, and the helper cannot
    /// remove the directory it is executing from.
    pub staging: PathBuf,
    pub plan_path: PathBuf,
    pub helper: PathBuf,
    pub roots: PurgeRoots,
}

impl StagedUninstall {
    /// Where the helper will write its account of the run.
    #[must_use]
    pub fn report_path(&self) -> PathBuf {
        let mut name = self
            .plan_path
            .file_name()
            .unwrap_or_default()
            .to_os_string();
        name.push(purge_exec::REPORT_FILE_SUFFIX);
        self.plan_path.with_file_name(name)
    }

    /// Does removing what this plan lists need more privilege than we have?
    #[must_use]
    pub fn needs_elevation(&self) -> bool {
        purge_exec::requires_elevation(&self.plan, dirs::home_dir().as_deref())
    }
}

/// Build the plan and stage everything the helper will need.
///
/// Read-only with respect to the installation: the only writes are into a
/// fresh temp directory. Nothing here is destructive, so a caller may stage,
/// inspect the plan, and drop it.
pub fn stage(
    data_dir: &Path,
    tier: PurgeTier,
    prefix: Option<&Path>,
) -> anyhow::Result<StagedUninstall> {
    let plan = purge::build_plan(
        &PlanRequest::new(data_dir.to_path_buf(), tier).with_prefix(prefix.map(Path::to_path_buf)),
    );

    let staging = create_staging_dir()?;
    let plan_path = staging.join(purge_exec::PLAN_FILE);
    std::fs::write(
        &plan_path,
        serde_json::to_vec_pretty(&plan).context("Cannot serialise the purge plan")?,
    )
    .with_context(|| format!("Cannot write the purge plan to {}", plan_path.display()))?;

    let helper = stage_helper_binary(&staging)?;

    Ok(StagedUninstall {
        plan,
        staging,
        plan_path,
        helper,
        roots: PurgeRoots::from_env(data_dir, prefix),
    })
}

/// Start the helper and return once it is running (or once the user has
/// declined to elevate it).
///
/// The caller must exit afterwards: the helper waits for `parent_pid` to
/// disappear before it touches anything, and gives up rather than deleting
/// files out from under a live process.
pub fn launch(staged: &StagedUninstall, parent_pid: u32) -> anyhow::Result<()> {
    let args = helper_args(staged, parent_pid);

    // Elevation is a Windows concept here. On Unix a purge that reaches outside
    // the user's own tree fails per entry and says so in the report — which is
    // honest, and better than a privilege-escalation prompt this code has no
    // established pattern for.
    #[cfg(windows)]
    if staged.needs_elevation() {
        return crate::platform::spawn_elevated_and_wait(
            &staged.helper,
            &args,
            ELEVATION_PROMPT_TIMEOUT,
        );
    }

    crate::platform::spawn_detached(&staged.helper, &args)
}

/// The helper's command line.
///
/// Split out so the argument list — the channel the containment set travels on
/// — can be asserted without spawning anything.
#[must_use]
pub fn helper_args(staged: &StagedUninstall, parent_pid: u32) -> Vec<String> {
    let mut args = vec![
        "purge-exec".to_string(),
        "--plan".to_string(),
        staged.plan_path.to_string_lossy().into_owned(),
        "--pid".to_string(),
        parent_pid.to_string(),
    ];
    for root in staged.roots.paths() {
        args.push("--root".to_string());
        args.push(root.to_string_lossy().into_owned());
    }
    args
}

/// A fresh, unpredictably named directory that only this user can enter.
///
/// `create_dir` rather than `create_dir_all`: it fails if the path already
/// exists, so a pre-created directory owned by someone else cannot become the
/// place we write the plan into.
///
/// Shared with the in-process path (`purge_exec::run_uninstall`), which needs
/// the same durable place outside the tree it is about to remove — only
/// without a helper copy, since nothing is handed off there.
pub(crate) fn create_staging_dir() -> anyhow::Result<PathBuf> {
    let suffix: u64 = rand::random();
    let path = std::env::temp_dir().join(format!(
        "clotocore-uninstall-{}-{suffix:016x}",
        std::process::id()
    ));
    std::fs::create_dir(&path).with_context(|| {
        format!(
            "Cannot create the uninstall staging directory {}",
            path.display()
        )
    })?;
    restrict_to_owner(&path);
    Ok(path)
}

/// Narrow the staging directory to its owner where the platform expresses that
/// as a mode. Best-effort: the plan's own containment floor is what the
/// executor relies on, and this only shrinks the window in which the plan could
/// be rewritten at all.
#[cfg(unix)]
fn restrict_to_owner(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn restrict_to_owner(_path: &Path) {}

/// Copy the running binary into the staging directory.
///
/// The helper has to be a *copy*: on Windows the original is locked while this
/// process lives, and it is frequently one of the things the plan removes.
fn stage_helper_binary(staging: &Path) -> anyhow::Result<PathBuf> {
    let current = std::env::current_exe().context("Cannot determine the running binary's path")?;
    let name = current
        .file_name()
        .map_or_else(|| PathBuf::from("clotocore-uninstall"), PathBuf::from);
    let helper = staging.join(name);
    std::fs::copy(&current, &helper)
        .with_context(|| format!("Cannot copy {} to {}", current.display(), helper.display()))?;
    crate::platform::set_executable_permission(&helper)
        .with_context(|| format!("Cannot make {} executable", helper.display()))?;
    Ok(helper)
}

/// Remove a staging directory. Only used when the launch never happened —
/// once the helper is running, the directory is its working set.
pub fn discard(staged: StagedUninstall) {
    if let Err(e) = std::fs::remove_dir_all(&staged.staging) {
        tracing::debug!(
            "uninstall staging directory {} not cleaned up: {e}",
            staged.staging.display()
        );
    }
}

/// Reject a tier that arrived from an API caller.
pub fn parse_tier(level: u8) -> anyhow::Result<PurgeTier> {
    PurgeTier::from_level(level)
        .ok_or_else(|| anyhow::anyhow!("Invalid scope tier {level}; expected 1, 2, 3 or 4"))
}

/// Refuse a prefix that is not a usable installation directory.
///
/// The prefix widens the containment set, so it is the one field of an
/// uninstall request that can make the executor willing to touch somewhere new.
/// It is not a capability — the request already carries the admin key — but a
/// caller that sends `/` should get an error rather than a plan.
pub fn checked_prefix(prefix: Option<PathBuf>) -> anyhow::Result<Option<PathBuf>> {
    let Some(prefix) = prefix else {
        return Ok(None);
    };
    if !prefix.is_absolute() {
        bail!(
            "Install prefix must be an absolute path: {}",
            prefix.display()
        );
    }
    if purge::is_filesystem_root(&prefix) {
        bail!(
            "Install prefix must not be a filesystem root: {}",
            prefix.display()
        );
    }
    if prefix
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        bail!(
            "Install prefix must not contain a parent-directory component: {}",
            prefix.display()
        );
    }
    Ok(Some(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn staged_in(dir: &Path) -> StagedUninstall {
        stage(dir, PurgeTier::Everything, None).expect("staging only writes to a temp dir")
    }

    #[test]
    fn staging_writes_a_readable_plan_and_a_runnable_helper() {
        let data = tempfile::tempdir().unwrap();
        let staged = staged_in(data.path());

        assert!(staged.plan_path.is_file(), "the plan must be on disk");
        assert!(staged.helper.is_file(), "the helper must be a real copy");
        let text = std::fs::read_to_string(&staged.plan_path).unwrap();
        let round_trip: PurgePlan = serde_json::from_str(&text)
            .expect("the helper parses this file with serde; so must the test");
        assert_eq!(round_trip.plan_version, staged.plan.plan_version);

        let current = std::env::current_exe().unwrap();
        assert_eq!(
            std::fs::metadata(&staged.helper).unwrap().len(),
            std::fs::metadata(&current).unwrap().len(),
            "the helper is a copy of the running binary, not a stub"
        );

        discard(staged);
    }

    #[test]
    fn the_containment_roots_travel_as_arguments() {
        // The whole point of the roots is that they do not go through the plan
        // file. If they ever moved in there, an attacker who can rewrite the
        // file could widen them, and the file's integrity would silently become
        // the entire boundary again.
        let data = tempfile::tempdir().unwrap();
        let staged = staged_in(data.path());

        let args = helper_args(&staged, 4242);
        assert_eq!(args[0], "purge-exec");
        assert!(args.contains(&"--pid".to_string()) && args.contains(&"4242".to_string()));
        assert!(
            !staged.roots.is_empty(),
            "a staged uninstall with no roots would refuse every entry"
        );
        for root in staged.roots.paths() {
            assert!(
                args.contains(&root.to_string_lossy().into_owned()),
                "{} must be passed on the command line",
                root.display()
            );
        }

        let plan_text = std::fs::read_to_string(&staged.plan_path).unwrap();
        assert!(
            !plan_text.contains("\"roots\""),
            "the plan file must not carry the containment set"
        );

        discard(staged);
    }

    #[test]
    fn each_staging_directory_is_its_own() {
        // Two runs must not share a directory: the second would overwrite the
        // first's plan while its helper was still reading it.
        let data = tempfile::tempdir().unwrap();
        let first = staged_in(data.path());
        let second = staged_in(data.path());
        assert_ne!(first.staging, second.staging);
        discard(first);
        discard(second);
    }

    #[test]
    fn the_report_path_is_where_the_executor_writes() {
        let data = tempfile::tempdir().unwrap();
        let staged = staged_in(data.path());
        let expected = staged
            .plan_path
            .with_file_name(format!("purge-plan.json{}", purge_exec::REPORT_FILE_SUFFIX));
        assert_eq!(staged.report_path(), expected);
        discard(staged);
    }

    #[test]
    fn a_prefix_that_could_widen_the_roots_is_checked() {
        // Absoluteness is platform-shaped: `/opt/cloto` is not an absolute path
        // on Windows, so the accepted case has to be written per platform or
        // the assertion tests the wrong thing on half the CI matrix.
        let (good, traversal) = if cfg!(windows) {
            (
                r"C:\Program Files\ClotoCore",
                r"C:\Program Files\..\..\Windows",
            )
        } else {
            ("/opt/cloto", "/opt/cloto/../../etc")
        };

        assert!(checked_prefix(None).unwrap().is_none());
        assert!(checked_prefix(Some(PathBuf::from("relative/dir"))).is_err());
        assert!(checked_prefix(Some(PathBuf::from("/"))).is_err());
        assert!(checked_prefix(Some(PathBuf::from(r"C:\"))).is_err());
        assert!(checked_prefix(Some(PathBuf::from(traversal))).is_err());
        assert_eq!(
            checked_prefix(Some(PathBuf::from(good))).unwrap().unwrap(),
            PathBuf::from(good)
        );
    }

    #[test]
    fn tiers_outside_the_scale_are_rejected() {
        assert_eq!(parse_tier(1).unwrap(), PurgeTier::Application);
        assert_eq!(parse_tier(4).unwrap(), PurgeTier::Everything);
        assert!(parse_tier(0).is_err());
        assert!(parse_tier(5).is_err());
    }
}
