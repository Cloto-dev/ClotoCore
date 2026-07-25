//! Purge plan — the capability boundary of the uninstall path
//! (DEFENDER_DESIGN.md §7).
//!
//! This module only *enumerates*. It never deletes: a plan is a concrete,
//! reviewable list of real paths with real sizes, produced from the install
//! receipt plus the artifacts the receipt cannot know about (OS service
//! registrations, webview profiles, stray legacy data directories). The
//! executor that consumes a plan is plan-bound by design (§8.5) — it has no
//! enumeration logic of its own, so anything absent from a plan is
//! structurally unreachable from the uninstall path, and anything wrongly
//! present in one will eventually be deleted. Both halves of that asymmetry
//! are decided here.
//!
//! Scope tiers are cumulative and conservative: tier 1 removes the
//! application only, and each higher tier adds a category the user has to opt
//! into explicitly (§7). The central invariant is **containment**: a
//! directory may only be listed at a tier that also covers everything inside
//! it. Without it, listing an application-tier directory would delete the
//! user data that happens to live under it — the CLI installer puts the
//! database, `.env` and seal key inside the install prefix, so this is the
//! normal layout, not a corner case.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::defender::footprint::{self, EntryKind, Receipt};

pub const PLAN_VERSION: u32 = 1;

/// Guard against a pathological walk (a data dir that swallowed a mount
/// point). Sizing stops here and the entry is reported as `size_truncated`
/// rather than silently under-reported.
const SIZE_WALK_LIMIT: usize = 200_000;

// ── Tiers ──

/// Cumulative removal scope (§7). `Application` is the default and the
/// narrowest; every higher tier is a superset of the ones below it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PurgeTier {
    /// Binary, app bundle, install prefix, OS service, autostart. The
    /// default: the narrowest scope a user can pick is also the one they get
    /// by omission.
    #[default]
    Application,
    /// \+ user data: databases, seal key, `.env`, attachments, avatars.
    UserData,
    /// \+ heavy assets: models, speech assets, downloaded runtimes, MCP
    /// servers and their venv.
    Assets,
    /// \+ everything else: data-directory containers, the receipt, webview
    /// profiles, uninstall registry keys.
    Everything,
}

impl PurgeTier {
    #[must_use]
    pub fn level(self) -> u8 {
        match self {
            Self::Application => 1,
            Self::UserData => 2,
            Self::Assets => 3,
            Self::Everything => 4,
        }
    }

    #[must_use]
    pub fn from_level(level: u8) -> Option<Self> {
        match level {
            1 => Some(Self::Application),
            2 => Some(Self::UserData),
            3 => Some(Self::Assets),
            4 => Some(Self::Everything),
            _ => None,
        }
    }

    /// Does a plan for `self` include entries classified as `entry`?
    #[must_use]
    pub fn includes(self, entry: Self) -> bool {
        entry.level() <= self.level()
    }
}

// ── Plan ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PurgeKind {
    File,
    Dir,
    /// OS service registration (systemd unit, launchd label, Windows service).
    Service,
    /// Windows uninstall registry key.
    Registry,
}

/// Where the entry came from, so a reviewer can tell a recorded footprint
/// from something discovered by scanning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PurgeSource {
    /// Listed in the install receipt.
    Receipt,
    /// Platform artifact the receipt does not track (webview profile,
    /// launchd plist, uninstall registry key).
    Platform,
    /// Stray data directory found by scanning, not recorded by any install —
    /// possibly another installation's data.
    Legacy,
}

impl PurgeSource {
    /// Short label for human-facing renderings, so "recorded" and "found by
    /// scanning" are distinguishable without reading JSON.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Receipt => "recorded",
            Self::Platform => "platform",
            Self::Legacy => "found by scan",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PurgeEntry {
    pub id: String,
    pub kind: PurgeKind,
    /// Absolute filesystem path for `File` / `Dir`, registry path for
    /// `Registry`. Always absolute: the executor runs detached from a temp
    /// directory (§7), so a relative path would resolve somewhere else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Service name for `Service` entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The tier this entry is removed at. For directories this is the
    /// *effective* tier — the widest tier of anything inside it.
    pub tier: PurgeTier,
    pub source: PurgeSource,
    /// Bytes on disk, for `File` / `Dir` entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    /// The size walk hit `SIZE_WALK_LIMIT`; `size_bytes` is a lower bound.
    #[serde(default, skip_serializing_if = "is_false")]
    pub size_truncated: bool,
    /// The path exists but could not be read (permissions). It stays in the
    /// plan: an elevated executor may well be able to remove it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub unreadable: bool,
    /// Holds credentials (seal key, `.env`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub secret: bool,
    /// A directory that contains secrets which collapsed into it. Without
    /// this, the widest tier — the one that removes the whole container —
    /// would be the only one that fails to warn about them.
    #[serde(default, skip_serializing_if = "is_false")]
    pub covers_secret: bool,
}

impl PurgeEntry {
    /// Does removing this entry destroy credentials, directly or by
    /// containment?
    #[must_use]
    pub fn destroys_secret(&self) -> bool {
        self.secret || self.covers_secret
    }
}

/// A candidate that was considered and left out, with the reason. Kept in the
/// plan because "we looked and it was not there" is information the user
/// needs to trust the enumeration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedEntry {
    pub id: String,
    pub reason: SkipReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// The path does not exist on this machine.
    Absent,
    /// Classified above the requested tier (for a directory, something inside
    /// it is).
    AboveTier,
    /// Contained in another entry that is already being removed.
    CoveredByParent,
    /// Refused by the plan's own floor: a filesystem root, or a path that
    /// could not be made absolute.
    Unsafe,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PurgePlan {
    pub plan_version: u32,
    /// Version of the binary that generated the plan.
    pub app_version: String,
    pub generated_at: String,
    pub tier: PurgeTier,
    pub data_dir: String,
    pub entries: Vec<PurgeEntry>,
    pub skipped: Vec<SkippedEntry>,
    /// Honest statements about the limits of this enumeration (§7
    /// "Boundaries"), rendered verbatim in every surface.
    pub notes: Vec<String>,
}

impl PurgePlan {
    /// Total bytes the plan would free (entries without a size contribute 0).
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.entries.iter().filter_map(|e| e.size_bytes).sum()
    }

    /// Is the total a lower bound rather than an exact figure?
    #[must_use]
    pub fn total_truncated(&self) -> bool {
        self.entries.iter().any(|e| e.size_truncated)
    }

    #[must_use]
    pub fn contains_secret(&self) -> bool {
        self.entries.iter().any(PurgeEntry::destroys_secret)
    }
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(b: &bool) -> bool {
    !*b
}

// ── Classification ──

/// Which tier a receipt entry belongs to, by id (§7).
///
/// Unknown ids fall back to `UserData` deliberately: an unrecognised entry is
/// never removed by the default tier-1 uninstall, which errs toward leaving
/// something behind rather than deleting something unclassified. Containment
/// carries that guarantee to directories — a tier-1 directory holding an
/// unknown entry is itself promoted out of tier 1.
#[must_use]
pub fn classify(id: &str) -> PurgeTier {
    if let Some(tier) = classify_exact(id) {
        return tier;
    }
    // MCP servers are installed per-server under the servers root.
    if id.starts_with("mcp:") {
        return PurgeTier::Assets;
    }
    PurgeTier::UserData
}

fn classify_exact(id: &str) -> Option<PurgeTier> {
    Some(match id {
        // Tier 1 — the application itself.
        "binary" | "app_bundle" | "install_prefix" | "install_scripts" | "service"
        | "autostart" => PurgeTier::Application,
        // Tier 2 — what the user created or the install personalised.
        "db" | "seal_key" | "env" | "setup_marker" | "attachments" | "avatars" | "vrm" | "logs"
        | "tmp" | "mcp_config" => PurgeTier::UserData,
        // Tier 3 — large, re-downloadable assets and third-party runtimes.
        "mcp_servers_root" | "mcp_sandbox" | "bin" | "models" | "voicevox" | "speech" => {
            PurgeTier::Assets
        }
        // Tier 4 — ledgers and the containers themselves. `install_data` is
        // the CLI installer's data directory: a container, like `data_dir`.
        "data_dir" | "install_data" | "receipt" | "webview" | "registry_uninstall" => {
            PurgeTier::Everything
        }
        _ => return None,
    })
}

// ── Probe roots (the injection seam) ──

/// The directories enumeration probes outside `data_dir`. Injectable so a
/// test can describe a whole machine layout instead of reading the developer's
/// real home directory — the plan builder used to walk it, which made results
/// host-dependent and, on a machine with a production install, slow.
#[derive(Debug, Clone, Default)]
pub struct ProbeRoots {
    pub home: Option<PathBuf>,
    /// Platform user-data dir (`dirs::data_dir`).
    pub platform_data: Option<PathBuf>,
    /// Platform cache dir (`dirs::cache_dir`).
    pub platform_cache: Option<PathBuf>,
    /// Platform local-data dir (`dirs::data_local_dir`, Windows webview).
    pub platform_local: Option<PathBuf>,
    /// Directory holding the running binary.
    pub exe_dir: Option<PathBuf>,
}

impl ProbeRoots {
    /// The real machine.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            home: dirs::home_dir(),
            platform_data: dirs::data_dir(),
            platform_cache: dirs::cache_dir(),
            platform_local: dirs::data_local_dir(),
            exe_dir: Some(crate::config::exe_dir()),
        }
    }
}

/// Everything the enumeration needs. Built with `PlanRequest::new` for the
/// real machine; constructed field-by-field in tests.
#[derive(Debug, Clone)]
pub struct PlanRequest {
    pub data_dir: PathBuf,
    pub tier: PurgeTier,
    /// CLI install prefix, when the caller has one (`clotocore uninstall
    /// --prefix`). Without it a plan says nothing about a prefix install that
    /// left no receipt.
    pub prefix: Option<PathBuf>,
    pub roots: ProbeRoots,
}

impl PlanRequest {
    #[must_use]
    pub fn new(data_dir: PathBuf, tier: PurgeTier) -> Self {
        Self {
            data_dir,
            tier,
            prefix: None,
            roots: ProbeRoots::from_env(),
        }
    }

    #[must_use]
    pub fn with_prefix(mut self, prefix: Option<PathBuf>) -> Self {
        self.prefix = prefix;
        self
    }
}

// ── Enumeration ──

/// Build a purge plan, reading the receipt in `req.data_dir` and probing the
/// platform artifacts the receipt does not track.
///
/// Read-only: nothing on disk is modified.
#[must_use]
pub fn build_plan(req: &PlanRequest) -> PurgePlan {
    let receipt = footprint::load(&req.data_dir);
    let candidates = collect_candidates(req, receipt.as_ref());
    finish_plan(req, candidates, receipt.as_ref())
}

/// A candidate before existence checks, tier promotion and nesting collapse.
#[derive(Debug, Clone)]
struct Candidate {
    id: String,
    kind: PurgeKind,
    path: Option<PathBuf>,
    name: Option<String>,
    tier: PurgeTier,
    source: PurgeSource,
    secret: bool,
}

impl Candidate {
    fn path_entry(
        id: impl Into<String>,
        kind: PurgeKind,
        path: PathBuf,
        tier: PurgeTier,
        source: PurgeSource,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            path: Some(path),
            name: None,
            tier,
            source,
            secret: false,
        }
    }
}

fn collect_candidates(req: &PlanRequest, receipt: Option<&Receipt>) -> Vec<Candidate> {
    let mut out = Vec::new();

    if let Some(receipt) = receipt {
        for entry in &receipt.entries {
            let kind = match entry.kind {
                EntryKind::File => PurgeKind::File,
                EntryKind::Dir => PurgeKind::Dir,
                EntryKind::Service => PurgeKind::Service,
            };
            out.push(Candidate {
                id: entry.id.clone(),
                kind,
                path: entry.path.as_ref().map(PathBuf::from),
                name: entry.name.clone(),
                tier: classify(&entry.id),
                source: PurgeSource::Receipt,
                secret: entry.secret,
            });
        }
    }

    // A caller-supplied prefix is what the non-plan `uninstall` path would
    // remove, so a dry run has to account for it even when no receipt exists.
    if let Some(prefix) = &req.prefix {
        out.push(Candidate::path_entry(
            "install_prefix",
            PurgeKind::Dir,
            prefix.clone(),
            PurgeTier::Application,
            PurgeSource::Receipt,
        ));
    }

    out.extend(platform_candidates(&req.roots));
    out.extend(legacy_candidates(&req.data_dir, &req.roots));
    out
}

/// Artifacts outside the receipt's reach (§7): the launchd plist, the systemd
/// unit, the webview profile, the Windows uninstall keys.
fn platform_candidates(roots: &ProbeRoots) -> Vec<Candidate> {
    let mut out = Vec::new();

    if cfg!(target_os = "macos") {
        if let Some(home) = &roots.home {
            out.push(Candidate::path_entry(
                "autostart",
                PurgeKind::File,
                home.join("Library/LaunchAgents/com.cloto.system.plist"),
                PurgeTier::Application,
                PurgeSource::Platform,
            ));
            for (idx, rel) in [
                "Library/WebKit/com.cloto.app",
                "Library/Caches/com.cloto.app",
                "Library/Application Support/com.cloto.app",
                "Library/Saved Application State/com.cloto.app.savedState",
            ]
            .iter()
            .enumerate()
            {
                out.push(Candidate::path_entry(
                    format!("webview_{idx}"),
                    PurgeKind::Dir,
                    home.join(rel),
                    PurgeTier::Everything,
                    PurgeSource::Platform,
                ));
            }
        }
    }

    if cfg!(target_os = "linux") {
        // The unit lives outside $HOME, so it must not be gated on one — a
        // service account with no HOME would otherwise leave it unremovable.
        out.push(Candidate::path_entry(
            "autostart",
            PurgeKind::File,
            PathBuf::from("/etc/systemd/system/cloto.service"),
            PurgeTier::Application,
            PurgeSource::Platform,
        ));
        for (idx, dir) in [&roots.platform_data, &roots.platform_cache]
            .into_iter()
            .enumerate()
        {
            if let Some(dir) = dir {
                out.push(Candidate::path_entry(
                    format!("webview_{idx}"),
                    PurgeKind::Dir,
                    dir.join("com.cloto.app"),
                    PurgeTier::Everything,
                    PurgeSource::Platform,
                ));
            }
        }
    }

    if cfg!(target_os = "windows") {
        if let Some(local) = &roots.platform_local {
            out.push(Candidate::path_entry(
                "webview_0",
                PurgeKind::Dir,
                local.join("com.cloto.app").join("EBWebView"),
                PurgeTier::Everything,
                PurgeSource::Platform,
            ));
        }
        // `ClotoCore` is the current key (install.ps1 / installer/uninstall.ps1);
        // `cloto-system` is the pre-rename one that installer.nsh still reads,
        // and an upgraded machine can carry both. Which hive holds a key
        // depends on per-machine vs per-user install, so all four are
        // candidates and the executor removes whichever exist.
        for key in ["ClotoCore", "cloto-system"] {
            for hive in ["HKLM", "HKCU"] {
                out.push(Candidate {
                    id: format!("registry_uninstall_{}_{key}", hive.to_lowercase()),
                    kind: PurgeKind::Registry,
                    path: Some(PathBuf::from(format!(
                        r"{hive}\Software\Microsoft\Windows\CurrentVersion\Uninstall\{key}"
                    ))),
                    name: None,
                    tier: PurgeTier::Everything,
                    source: PurgeSource::Platform,
                    secret: false,
                });
            }
        }
    }

    out
}

/// Stray data directories that hold a database the running binary never reads
/// (the same drift the `legacy_data_dir_drift` check reports; repair leaves
/// them alone on purpose, so purge is the only path that offers removal).
///
/// Classified at `Everything`, like any other data-directory container: what
/// is inside one spans every tier, and it may belong to a *different*
/// installation, so it must never go with a narrower scope.
fn legacy_candidates(data_dir: &Path, roots: &ProbeRoots) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    if let Some(exe_dir) = &roots.exe_dir {
        candidates.push(exe_dir.join("data"));
    }
    if let Some(platform_data) = &roots.platform_data {
        candidates.push(platform_data.join(crate::config::APP_DATA_DIR_NAME));
    }

    // Shared with the doctor check so the two can never disagree about what
    // counts as drift (§1: health knowing about something uninstall forgets
    // is the failure this subsystem exists to prevent).
    crate::defender::checks::drift_hits(data_dir, &candidates)
        .into_iter()
        .enumerate()
        .map(|(idx, hit)| {
            Candidate::path_entry(
                format!("legacy_data_dir_{idx}"),
                PurgeKind::Dir,
                hit,
                PurgeTier::Everything,
                PurgeSource::Legacy,
            )
        })
        .collect()
}

fn finish_plan(
    req: &PlanRequest,
    candidates: Vec<Candidate>,
    receipt: Option<&Receipt>,
) -> PurgePlan {
    let mut skipped = Vec::new();

    let present = probe_existence(candidates, &mut skipped);
    let deduped = dedupe_by_path(present);
    let promoted = promote_containers(deduped);
    let in_tier = filter_by_tier(promoted, req.tier, &mut skipped);
    let (collapsed, covered) = collapse_nested(in_tier);
    skipped.extend(covered);

    let entries = order_for_removal(collapsed.into_iter().map(measure_entry).collect());

    PurgePlan {
        plan_version: PLAN_VERSION,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        tier: req.tier,
        data_dir: req.data_dir.display().to_string(),
        entries,
        skipped,
        notes: notes(receipt, req),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Existence {
    Present,
    /// The path is there but cannot be stat'd (permissions).
    Unreadable,
    Missing,
}

/// Classify a stat result.
///
/// Only `NotFound` means absent. A permission error keeps the entry in the
/// plan, because the executor may run elevated (§7) — reading "cannot stat"
/// as "not there" would silently leave an unreadable `.env` behind after an
/// uninstall that claimed to be complete.
fn existence(result: Result<(), std::io::ErrorKind>) -> Existence {
    match result {
        Ok(()) => Existence::Present,
        Err(std::io::ErrorKind::NotFound) => Existence::Missing,
        Err(_) => Existence::Unreadable,
    }
}

/// A candidate that survived existence probing, before tier decisions.
#[derive(Debug, Clone)]
struct Present {
    id: String,
    kind: PurgeKind,
    path: Option<PathBuf>,
    name: Option<String>,
    tier: PurgeTier,
    source: PurgeSource,
    secret: bool,
    covers_secret: bool,
    unreadable: bool,
}

/// Drop what is not on disk, and refuse paths the plan must never carry.
///
/// "Cannot stat" is not "absent": a permission error keeps the entry, because
/// the executor may run elevated. Treating it as absent would quietly leave
/// an unreadable `.env` behind after a "complete" uninstall.
fn probe_existence(candidates: Vec<Candidate>, skipped: &mut Vec<SkippedEntry>) -> Vec<Present> {
    let mut out = Vec::new();
    for candidate in candidates {
        let (path, unreadable) = match (candidate.kind, candidate.path.as_ref()) {
            (PurgeKind::File | PurgeKind::Dir, Some(path)) => {
                let Some(abs) = absolutize(path) else {
                    skipped.push(SkippedEntry {
                        id: candidate.id,
                        reason: SkipReason::Unsafe,
                        path: Some(path.display().to_string()),
                    });
                    continue;
                };
                if is_filesystem_root(&abs) {
                    skipped.push(SkippedEntry {
                        id: candidate.id,
                        reason: SkipReason::Unsafe,
                        path: Some(abs.display().to_string()),
                    });
                    continue;
                }
                let probe = existence(
                    std::fs::symlink_metadata(&abs)
                        .map(|_| ())
                        .map_err(|e| e.kind()),
                );
                match probe {
                    Existence::Present => (Some(abs), false),
                    Existence::Unreadable => (Some(abs), true),
                    Existence::Missing => {
                        skipped.push(SkippedEntry {
                            id: candidate.id,
                            reason: SkipReason::Absent,
                            path: Some(abs.display().to_string()),
                        });
                        continue;
                    }
                }
            }
            // Services and registry keys are removed through OS calls; there
            // is nothing to stat, and the executor no-ops when absent.
            _ => (candidate.path.clone(), false),
        };

        out.push(Present {
            id: candidate.id,
            kind: candidate.kind,
            path,
            name: candidate.name,
            tier: candidate.tier,
            source: candidate.source,
            secret: candidate.secret,
            covers_secret: false,
            unreadable,
        });
    }
    out
}

/// Collapse duplicate paths (the receipt and the legacy scan can name the
/// same directory). The survivor keeps the *widest* tier and any secret flag —
/// merging toward the narrower tier would let a duplicate smuggle a container
/// into a lower scope.
fn dedupe_by_path(entries: Vec<Present>) -> Vec<Present> {
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    let mut out: Vec<Present> = Vec::new();

    for entry in entries {
        let Some(key) = entry.path.as_ref().map(|p| dedupe_key(p)) else {
            out.push(entry);
            continue;
        };
        if let Some(&existing) = index.get(&key) {
            let winner = &mut out[existing];
            winner.tier = winner.tier.max(entry.tier);
            winner.secret |= entry.secret;
            winner.unreadable |= entry.unreadable;
            // Prefer the recorded identity over a scanned one.
            if winner.source != PurgeSource::Receipt && entry.source == PurgeSource::Receipt {
                winner.source = entry.source;
                winner.id = entry.id;
            }
        } else {
            index.insert(key, out.len());
            out.push(entry);
        }
    }
    out
}

/// Raise every directory to the widest tier found inside it.
///
/// This is the containment invariant. The CLI installer puts the database,
/// `.env` and the seal key *inside* the install prefix, so without promotion a
/// tier-1 ("application only") plan would list the prefix for recursive
/// removal while reporting the user's data as out of scope.
fn promote_containers(mut entries: Vec<Present>) -> Vec<Present> {
    let inner: Vec<(PathBuf, PurgeTier)> = entries
        .iter()
        .filter_map(|e| e.path.clone().map(|p| (p, e.tier)))
        .collect();

    for entry in &mut entries {
        if entry.kind != PurgeKind::Dir {
            continue;
        }
        let Some(dir) = entry.path.clone() else {
            continue;
        };
        for (path, tier) in &inner {
            if is_strict_descendant(path, &dir) {
                entry.tier = entry.tier.max(*tier);
            }
        }
    }
    entries
}

fn filter_by_tier(
    entries: Vec<Present>,
    tier: PurgeTier,
    skipped: &mut Vec<SkippedEntry>,
) -> Vec<Present> {
    let mut out = Vec::new();
    for entry in entries {
        if tier.includes(entry.tier) {
            out.push(entry);
        } else {
            skipped.push(SkippedEntry {
                id: entry.id,
                reason: SkipReason::AboveTier,
                path: entry.path.map(|p| p.display().to_string()),
            });
        }
    }
    out
}

/// Drop entries contained in a directory that is already being removed. Safe
/// only because `promote_containers` ran first: a surviving directory covers
/// everything inside it by construction. The child is reported as
/// `CoveredByParent` rather than dropped silently, its secret flag is
/// propagated to the parent, and its size stops double-counting.
fn collapse_nested(entries: Vec<Present>) -> (Vec<Present>, Vec<SkippedEntry>) {
    let dirs: Vec<PathBuf> = entries
        .iter()
        .filter(|e| e.kind == PurgeKind::Dir)
        .filter_map(|e| e.path.clone())
        .collect();

    let mut kept: Vec<Present> = Vec::new();
    let mut covered = Vec::new();
    let mut secret_parents: Vec<PathBuf> = Vec::new();

    for entry in entries {
        let Some(path) = entry.path.clone() else {
            kept.push(entry);
            continue;
        };
        if let Some(parent) = dirs.iter().find(|d| is_strict_descendant(&path, d)) {
            if entry.secret || entry.covers_secret {
                secret_parents.push(parent.clone());
            }
            covered.push(SkippedEntry {
                id: entry.id,
                reason: SkipReason::CoveredByParent,
                path: Some(path.display().to_string()),
            });
        } else {
            kept.push(entry);
        }
    }

    for entry in &mut kept {
        if let Some(path) = entry.path.as_ref() {
            if secret_parents.iter().any(|p| p == path) {
                entry.covers_secret = true;
            }
        }
    }

    (kept, covered)
}

fn measure_entry(entry: Present) -> PurgeEntry {
    let (size_bytes, size_truncated) = match (entry.kind, entry.path.as_ref(), entry.unreadable) {
        (PurgeKind::File | PurgeKind::Dir, Some(path), false) => {
            let (bytes, truncated) = measure(path);
            (Some(bytes), truncated)
        }
        _ => (None, false),
    };
    PurgeEntry {
        id: entry.id,
        kind: entry.kind,
        path: entry.path.map(|p| p.display().to_string()),
        name: entry.name,
        tier: entry.tier,
        source: entry.source,
        size_bytes,
        size_truncated,
        unreadable: entry.unreadable,
        secret: entry.secret,
        covers_secret: entry.covers_secret,
    }
}

fn notes(receipt: Option<&Receipt>, req: &PlanRequest) -> Vec<String> {
    let mut notes = vec![
        "Third-party MCP servers may write outside their install directory (their own caches and \
         configs). Only declared paths are enumerated here."
            .to_string(),
        "OS-level traces (prefetch, event logs, recently-used lists) are out of scope and are \
         never removed."
            .to_string(),
    ];
    if receipt.is_none() {
        notes.push(
            "No install receipt was found, so this plan comes from platform probing alone and is \
             almost certainly incomplete. Install receipts are written from the first boot of \
             0.6.8 onward."
                .to_string(),
        );
    }
    if req.prefix.is_none() {
        notes.push(
            "This plan covers the receipt and the platform artifacts of this installation. A CLI \
             install in a custom prefix that left no receipt is only covered when --prefix is \
             given."
                .to_string(),
        );
    }
    notes.push(
        "Windows registry keys and OS services are deregistered only if present; they are listed \
         without a size because there is nothing to measure."
            .to_string(),
    );
    notes
}

// ── Pure helpers (unit-tested) ──

/// Make a path absolute and lexically clean, without touching the filesystem
/// (`canonicalize` would resolve symlinks, and the target may be exactly what
/// the user wants removed).
///
/// Returns `None` when the path is relative and the current directory is
/// unavailable — a plan must never carry a relative path, because the
/// executor runs detached from a temp directory (§7) where it would resolve
/// somewhere else entirely.
fn absolutize(path: &Path) -> Option<PathBuf> {
    let base = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };

    let mut out = PathBuf::new();
    for component in base.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    Some(out)
}

/// A path with nothing above it — `/`, `C:\`, a bare UNC share. The plan
/// refuses these outright; no uninstall footprint is ever a filesystem root.
fn is_filesystem_root(path: &Path) -> bool {
    path.parent().is_none()
        || path
            .components()
            .all(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
}

/// Key used to detect two candidates naming the same path. Windows paths are
/// case-insensitive, so a case difference must not read as two entries.
fn dedupe_key(path: &Path) -> String {
    let raw = path.display().to_string();
    if cfg!(windows) {
        raw.to_lowercase()
    } else {
        raw
    }
}

/// Is `path` strictly inside `dir`? Equal paths are not descendants, so an
/// entry never suppresses or promotes itself.
fn is_strict_descendant(path: &Path, dir: &Path) -> bool {
    path != dir && path.starts_with(dir)
}

/// Deregistrations first, then deepest paths first.
///
/// Services must be deregistered *before* their unit or plist file is
/// deleted: `platform::uninstall_service` only unloads a launchd job when the
/// plist still exists, so removing the file first turns the deregistration
/// into a silent no-op and leaves the job loaded.
fn order_for_removal(mut entries: Vec<PurgeEntry>) -> Vec<PurgeEntry> {
    entries.sort_by(|a, b| {
        let rank = |e: &PurgeEntry| match e.kind {
            PurgeKind::Service | PurgeKind::Registry => 0,
            PurgeKind::File | PurgeKind::Dir => 1,
        };
        let depth = |e: &PurgeEntry| {
            e.path
                .as_ref()
                .map_or(0, |p| Path::new(p).components().count())
        };
        rank(a)
            .cmp(&rank(b))
            .then(depth(b).cmp(&depth(a)))
            .then(a.path.cmp(&b.path))
            .then(a.id.cmp(&b.id))
    });
    entries
}

/// Bytes under `path`, without following symlinks. Returns the size and
/// whether the walk was truncated at `SIZE_WALK_LIMIT`.
fn measure(path: &Path) -> (u64, bool) {
    measure_with_limit(path, SIZE_WALK_LIMIT)
}

fn measure_with_limit(path: &Path, limit: usize) -> (u64, bool) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return (0, false);
    };
    // A symlinked directory measures as the link itself: following it would
    // report (and later suggest removing) bytes that live elsewhere.
    if !meta.is_dir() {
        return (meta.len(), false);
    }

    let mut total = 0_u64;
    let mut visited = 0_usize;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            visited += 1;
            if visited > limit {
                return (total, true);
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                stack.push(entry.path());
            } else if !meta.is_symlink() {
                total = total.saturating_add(meta.len());
            }
        }
    }
    (total, false)
}

// ── CLI surface ──

/// `clotocore uninstall --plan` — render the purge plan for `tier` without
/// touching anything (§7: the dry-run enumeration is the first of the three
/// gates, and it is useful on its own for headless installs).
pub fn run_cli(tier: u8, prefix: Option<PathBuf>, json: bool) -> anyhow::Result<()> {
    let tier = PurgeTier::from_level(tier)
        .ok_or_else(|| anyhow::anyhow!("Invalid scope tier {tier}; expected 1, 2, 3 or 4"))?;
    let data_dir = crate::config::data_dir();
    let plan = build_plan(&PlanRequest::new(data_dir, tier).with_prefix(prefix));

    if json {
        println!("{}", serde_json::to_string_pretty(&plan)?);
    } else {
        print!("{}", render_text(&plan));
    }
    Ok(())
}

/// Human-readable rendering of a plan. Kept pure so the wording that the user
/// approves is itself testable.
#[must_use]
pub fn render_text(plan: &PurgePlan) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    let _ = writeln!(
        out,
        "ClotoCore uninstall plan (format v{}, generated by v{})",
        plan.plan_version, plan.app_version
    );
    let _ = writeln!(out, "Data directory: {}", plan.data_dir);
    let _ = writeln!(
        out,
        "Scope tier:     {} ({})",
        plan.tier.level(),
        tier_label(plan.tier)
    );
    let _ = writeln!(out);

    if plan.entries.is_empty() {
        let _ = writeln!(out, "  Nothing to remove at this tier.");
    } else {
        for entry in &plan.entries {
            let target = entry
                .path
                .clone()
                .or_else(|| entry.name.clone())
                .unwrap_or_else(|| "<unnamed>".to_string());
            let kind = match entry.kind {
                PurgeKind::File => "file",
                PurgeKind::Dir => "dir ",
                PurgeKind::Service => "svc ",
                PurgeKind::Registry => "reg ",
            };
            let size = entry.size_bytes.map_or_else(String::new, |b| {
                format!(
                    " ({}{})",
                    human_bytes(b),
                    if entry.size_truncated { "+" } else { "" }
                )
            });
            let mut flags = vec![entry.source.label().to_string()];
            if entry.destroys_secret() {
                flags.push("contains secrets".to_string());
            }
            if entry.unreadable {
                flags.push("unreadable — needs elevation".to_string());
            }
            let _ = writeln!(out, "  {kind} {target}{size}  [{}]", flags.join("; "));
        }
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "  {} item(s), {}{} total",
            plan.entries.len(),
            human_bytes(plan.total_bytes()),
            if plan.total_truncated() { "+" } else { "" }
        );
    }

    if !plan.skipped.is_empty() {
        let count = |reason: SkipReason| plan.skipped.iter().filter(|s| s.reason == reason).count();
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "  not listed: {} above this tier, {} not present, {} inside a directory already \
             listed, {} refused as unsafe",
            count(SkipReason::AboveTier),
            count(SkipReason::Absent),
            count(SkipReason::CoveredByParent),
            count(SkipReason::Unsafe)
        );
        for entry in plan
            .skipped
            .iter()
            .filter(|s| s.reason == SkipReason::AboveTier)
        {
            if let Some(path) = &entry.path {
                let _ = writeln!(out, "    above tier: {path}");
            }
        }
    }

    if !plan.notes.is_empty() {
        let _ = writeln!(out);
        for note in &plan.notes {
            let _ = writeln!(out, "  note: {note}");
        }
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "(plan only — nothing was removed)");
    out
}

fn tier_label(tier: PurgeTier) -> &'static str {
    match tier {
        PurgeTier::Application => "application only",
        PurgeTier::UserData => "application + user data",
        PurgeTier::Assets => "application + user data + assets and MCP servers",
        PurgeTier::Everything => "everything, including the data directory itself",
    }
}

fn human_bytes(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss)]
    let b = bytes as f64;
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", b / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MB", b / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", b / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defender::footprint::ReceiptEntry;

    /// A plan request that probes nothing outside the given data dir, so a
    /// test describes its own machine instead of the developer's.
    fn isolated(data_dir: &Path, tier: PurgeTier) -> PlanRequest {
        PlanRequest {
            data_dir: data_dir.to_path_buf(),
            tier,
            prefix: None,
            roots: ProbeRoots::default(),
        }
    }

    fn ids(plan: &PurgePlan) -> Vec<&str> {
        plan.entries.iter().map(|e| e.id.as_str()).collect()
    }

    fn skipped_for(plan: &PurgePlan, id: &str) -> Option<SkipReason> {
        plan.skipped.iter().find(|s| s.id == id).map(|s| s.reason)
    }

    fn touch(path: &Path, bytes: usize) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, vec![0_u8; bytes]).unwrap();
    }

    #[test]
    fn tiers_are_cumulative() {
        assert!(PurgeTier::Everything.includes(PurgeTier::Application));
        assert!(PurgeTier::UserData.includes(PurgeTier::UserData));
        assert!(!PurgeTier::Application.includes(PurgeTier::UserData));
        assert!(!PurgeTier::Assets.includes(PurgeTier::Everything));
        assert_eq!(PurgeTier::from_level(3), Some(PurgeTier::Assets));
        assert_eq!(PurgeTier::from_level(5), None);
        assert_eq!(PurgeTier::default(), PurgeTier::Application);
    }

    #[test]
    fn classification_covers_the_known_footprint() {
        assert_eq!(classify("binary"), PurgeTier::Application);
        assert_eq!(classify("app_bundle"), PurgeTier::Application);
        assert_eq!(classify("db"), PurgeTier::UserData);
        assert_eq!(classify("seal_key"), PurgeTier::UserData);
        assert_eq!(classify("mcp_servers_root"), PurgeTier::Assets);
        assert_eq!(classify("mcp:filesystem"), PurgeTier::Assets);
        assert_eq!(classify("data_dir"), PurgeTier::Everything);
        assert_eq!(classify("install_data"), PurgeTier::Everything);
        assert_eq!(classify("receipt"), PurgeTier::Everything);
    }

    #[test]
    fn unknown_ids_default_to_user_data_not_application() {
        // The default must never be tier 1: an unrecognised entry would then
        // be removed by the narrowest uninstall the user can pick.
        assert_eq!(
            classify("something_new_in_a_later_version"),
            PurgeTier::UserData
        );
        assert!(!PurgeTier::Application.includes(classify("something_new_in_a_later_version")));
    }

    // ── Containment ──

    #[test]
    fn tier_one_does_not_remove_a_prefix_that_holds_user_data() {
        // The CLI installer's real layout: binary, scripts, .env and the data
        // directory all live inside the prefix (installer.rs).
        let root = tempfile::tempdir().unwrap();
        let prefix = root.path().join("opt/cloto");
        let data = prefix.join("data");
        touch(&prefix.join("clotocore"), 10);
        touch(&prefix.join(".env"), 20);
        touch(&data.join("cloto_memories.db"), 100_000);
        touch(&data.join("seal.key"), 32);
        std::fs::create_dir_all(prefix.join("scripts")).unwrap();

        let receipt_dir = root.path().join("receipt");
        std::fs::create_dir_all(&receipt_dir).unwrap();
        footprint::record(
            &receipt_dir,
            vec![
                ReceiptEntry::file("binary", &prefix.join("clotocore")),
                ReceiptEntry::dir("install_prefix", &prefix),
                ReceiptEntry::dir("install_scripts", &prefix.join("scripts")),
                ReceiptEntry::dir("install_data", &data),
                ReceiptEntry::file("env", &prefix.join(".env")).secret(),
                ReceiptEntry::file("db", &data.join("cloto_memories.db")),
                ReceiptEntry::file("seal_key", &data.join("seal.key")).secret(),
            ],
        );

        let plan = build_plan(&isolated(&receipt_dir, PurgeTier::Application));
        assert!(
            !ids(&plan).contains(&"install_prefix"),
            "the prefix holds the database and the seal key; listing it at tier 1 would delete \
             them while the plan claims they are out of scope"
        );
        assert_eq!(
            skipped_for(&plan, "install_prefix"),
            Some(SkipReason::AboveTier)
        );
        assert!(ids(&plan).contains(&"binary"));
        assert!(ids(&plan).contains(&"install_scripts"));
        assert_eq!(
            plan.total_bytes(),
            10,
            "only the binary is in scope at tier 1"
        );
    }

    #[test]
    fn a_container_is_promoted_to_the_widest_tier_it_holds() {
        let root = tempfile::tempdir().unwrap();
        let prefix = root.path().join("prefix");
        touch(&prefix.join("bin/clotocore"), 1);
        touch(&prefix.join("data/cloto_memories.db"), 1);
        let receipt_dir = root.path().join("r");
        std::fs::create_dir_all(&receipt_dir).unwrap();
        footprint::record(
            &receipt_dir,
            vec![
                ReceiptEntry::dir("install_prefix", &prefix),
                ReceiptEntry::dir("install_data", &prefix.join("data")),
            ],
        );

        // install_data is a container (tier 4), so the prefix that holds it
        // cannot be removed before tier 4 either.
        for tier in [
            PurgeTier::Application,
            PurgeTier::UserData,
            PurgeTier::Assets,
        ] {
            let plan = build_plan(&isolated(&receipt_dir, tier));
            assert!(
                !ids(&plan).contains(&"install_prefix"),
                "prefix must not be listed at tier {}",
                tier.level()
            );
        }
        let plan = build_plan(&isolated(&receipt_dir, PurgeTier::Everything));
        assert!(ids(&plan).contains(&"install_prefix"));
    }

    #[test]
    fn an_unknown_entry_inside_a_tier_one_directory_still_survives_tier_one() {
        let root = tempfile::tempdir().unwrap();
        let prefix = root.path().join("prefix");
        touch(&prefix.join("mystery.dat"), 5);
        let receipt_dir = root.path().join("r");
        std::fs::create_dir_all(&receipt_dir).unwrap();
        footprint::record(
            &receipt_dir,
            vec![
                ReceiptEntry::dir("install_prefix", &prefix),
                ReceiptEntry::file("brand_new_thing", &prefix.join("mystery.dat")),
            ],
        );
        let plan = build_plan(&isolated(&receipt_dir, PurgeTier::Application));
        assert!(
            plan.entries.is_empty(),
            "an unclassified entry must survive a scope the user did not knowingly widen, \
             including by containment: {:?}",
            ids(&plan)
        );
    }

    #[test]
    fn no_listed_directory_hides_an_above_tier_entry() {
        // The general form of the containment invariant, asserted over a
        // realistic receipt at every tier.
        let root = tempfile::tempdir().unwrap();
        let prefix = root.path().join("prefix");
        touch(&prefix.join("clotocore"), 1);
        touch(&prefix.join(".env"), 1);
        touch(&prefix.join("data/cloto_memories.db"), 1);
        touch(&prefix.join("data/mcp-servers/x/f"), 1);
        let receipt_dir = root.path().join("r");
        std::fs::create_dir_all(&receipt_dir).unwrap();
        footprint::record(
            &receipt_dir,
            vec![
                ReceiptEntry::file("binary", &prefix.join("clotocore")),
                ReceiptEntry::dir("install_prefix", &prefix),
                ReceiptEntry::file("env", &prefix.join(".env")).secret(),
                ReceiptEntry::dir("install_data", &prefix.join("data")),
                ReceiptEntry::file("db", &prefix.join("data/cloto_memories.db")),
                ReceiptEntry::dir("mcp_servers_root", &prefix.join("data/mcp-servers")),
            ],
        );

        for level in 1..=4 {
            let tier = PurgeTier::from_level(level).unwrap();
            let plan = build_plan(&isolated(&receipt_dir, tier));
            let listed_dirs: Vec<PathBuf> = plan
                .entries
                .iter()
                .filter(|e| e.kind == PurgeKind::Dir)
                .filter_map(|e| e.path.as_ref().map(PathBuf::from))
                .collect();
            for skipped in &plan.skipped {
                if skipped.reason != SkipReason::AboveTier {
                    continue;
                }
                let Some(path) = skipped.path.as_ref().map(PathBuf::from) else {
                    continue;
                };
                assert!(
                    !listed_dirs.iter().any(|d| is_strict_descendant(&path, d)),
                    "tier {level}: {} is reported as out of scope but sits inside a directory \
                     the plan removes",
                    path.display()
                );
            }
        }
    }

    // ── Nesting and totals ──

    #[test]
    fn nested_entries_collapse_into_their_parent() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        touch(&data.join("cloto_memories.db"), 100);
        footprint::record(
            &data,
            vec![
                ReceiptEntry::dir("data_dir", &data),
                ReceiptEntry::file("db", &data.join("cloto_memories.db")),
            ],
        );

        let plan = build_plan(&isolated(&data, PurgeTier::Everything));
        assert!(ids(&plan).contains(&"data_dir"));
        assert!(!ids(&plan).contains(&"db"));
        assert_eq!(skipped_for(&plan, "db"), Some(SkipReason::CoveredByParent));

        let data_dir_size = plan
            .entries
            .iter()
            .find(|e| e.id == "data_dir")
            .and_then(|e| e.size_bytes)
            .unwrap();
        assert_eq!(
            plan.total_bytes(),
            data_dir_size,
            "the total must count the tree once"
        );
    }

    #[test]
    fn a_directory_does_not_suppress_itself() {
        assert!(!is_strict_descendant(Path::new("/d"), Path::new("/d")));
        assert!(is_strict_descendant(Path::new("/d/sub"), Path::new("/d")));
        // "/data-old" starts with the *string* "/data" but is not inside it.
        assert!(!is_strict_descendant(
            Path::new("/data-old"),
            Path::new("/data")
        ));
    }

    #[test]
    fn the_same_path_from_two_sources_is_listed_once() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        touch(&data.join("cloto_memories.db"), 50_000);
        let receipt_dir = root.path().join("r");
        std::fs::create_dir_all(&receipt_dir).unwrap();
        footprint::record(&receipt_dir, vec![ReceiptEntry::dir("install_data", &data)]);

        // The legacy scan finds the very same directory (exe_dir/data).
        let mut req = isolated(&receipt_dir, PurgeTier::Everything);
        req.roots.exe_dir = Some(root.path().to_path_buf());
        let plan = build_plan(&req);

        let listing: Vec<_> = plan
            .entries
            .iter()
            .filter(|e| e.path.as_deref() == Some(data.display().to_string().as_str()))
            .collect();
        assert_eq!(listing.len(), 1, "one path, one entry: {:?}", ids(&plan));
        assert_eq!(listing[0].source, PurgeSource::Receipt, "recorded wins");

        // The general form: no path may appear twice, or its bytes would be
        // counted twice in the total the user is shown.
        let mut seen = std::collections::BTreeSet::new();
        for entry in &plan.entries {
            if let Some(path) = &entry.path {
                assert!(
                    seen.insert(path.clone()),
                    "{path} is listed more than once, so its size is counted more than once"
                );
            }
        }
    }

    // ── Secrets ──

    #[test]
    fn secrets_are_flagged_at_every_tier_that_destroys_them() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        touch(&data.join("seal.key"), 32);
        touch(&data.join(".env"), 20);
        footprint::record(
            &data,
            vec![
                ReceiptEntry::dir("data_dir", &data),
                ReceiptEntry::file("seal_key", &data.join("seal.key")).secret(),
                ReceiptEntry::file("env", &data.join(".env")).secret(),
            ],
        );

        for tier in [
            PurgeTier::UserData,
            PurgeTier::Assets,
            PurgeTier::Everything,
        ] {
            let plan = build_plan(&isolated(&data, tier));
            assert!(
                plan.contains_secret(),
                "tier {} destroys the seal key, so the confirmation must say so",
                tier.level()
            );
            assert!(render_text(&plan).contains("contains secrets"));
        }
    }

    // ── Existence and safety floor ──

    #[test]
    fn plan_only_lists_what_exists() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("cloto_memories.db");
        touch(&db, 1);
        footprint::record(
            dir.path(),
            vec![
                ReceiptEntry::file("db", &db),
                ReceiptEntry::file("binary", &dir.path().join("gone-binary")),
            ],
        );

        let plan = build_plan(&isolated(dir.path(), PurgeTier::UserData));
        assert!(ids(&plan).contains(&"db"));
        assert!(
            !ids(&plan).contains(&"binary"),
            "a recorded path that no longer exists must not be listed as removable"
        );
        assert_eq!(skipped_for(&plan, "binary"), Some(SkipReason::Absent));
    }

    #[test]
    fn an_unreadable_path_stays_in_the_plan() {
        use std::io::ErrorKind;
        assert_eq!(existence(Ok(())), Existence::Present);
        assert_eq!(existence(Err(ErrorKind::NotFound)), Existence::Missing);
        // The executor may run elevated, so "I could not look" must not be
        // reported as "it is not there" — that is how an unreadable .env
        // survives a "complete" uninstall.
        assert_eq!(
            existence(Err(ErrorKind::PermissionDenied)),
            Existence::Unreadable
        );
        assert_eq!(existence(Err(ErrorKind::Other)), Existence::Unreadable);
    }

    #[test]
    fn merging_duplicates_keeps_the_wider_tier() {
        // The same directory reached by two probes must not be smuggled into
        // a narrower scope by whichever candidate happened to come first.
        let entry = |id: &str, tier: PurgeTier, source: PurgeSource, secret: bool| Present {
            id: id.into(),
            kind: PurgeKind::Dir,
            path: Some(PathBuf::from("/same")),
            name: None,
            tier,
            source,
            secret,
            covers_secret: false,
            unreadable: false,
        };

        let merged = dedupe_by_path(vec![
            entry("db", PurgeTier::UserData, PurgeSource::Legacy, false),
            entry(
                "data_dir",
                PurgeTier::Everything,
                PurgeSource::Receipt,
                true,
            ),
        ]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].tier, PurgeTier::Everything);
        assert!(merged[0].secret, "a secret flag must survive the merge");
        assert_eq!(merged[0].source, PurgeSource::Receipt);

        // Order must not change the outcome.
        let merged = dedupe_by_path(vec![
            entry(
                "data_dir",
                PurgeTier::Everything,
                PurgeSource::Receipt,
                false,
            ),
            entry("db", PurgeTier::UserData, PurgeSource::Legacy, false),
        ]);
        assert_eq!(merged[0].tier, PurgeTier::Everything);
    }

    #[test]
    fn a_filesystem_root_is_refused_outright() {
        assert!(is_filesystem_root(Path::new("/")));
        assert!(!is_filesystem_root(Path::new("/opt")));
        if cfg!(windows) {
            assert!(is_filesystem_root(Path::new(r"C:\")));
            assert!(!is_filesystem_root(Path::new(r"C:\Program Files")));
        }

        let dir = tempfile::tempdir().unwrap();
        footprint::record(
            dir.path(),
            vec![ReceiptEntry::dir("install_prefix", Path::new("/"))],
        );
        let plan = build_plan(&isolated(dir.path(), PurgeTier::Everything));
        assert_eq!(
            skipped_for(&plan, "install_prefix"),
            Some(SkipReason::Unsafe)
        );
    }

    #[test]
    fn every_listed_path_is_absolute_and_lexically_clean() {
        // The executor runs detached from a temp directory, so a relative
        // path in the plan would resolve somewhere else entirely.
        let dir = tempfile::tempdir().unwrap();
        let messy = dir.path().join("sub/../keep.dat");
        touch(&dir.path().join("keep.dat"), 4);
        footprint::record(
            &dir.path().join("r"),
            vec![ReceiptEntry::file("db", &messy)],
        );

        let plan = build_plan(&isolated(&dir.path().join("r"), PurgeTier::UserData));
        for entry in &plan.entries {
            if entry.kind == PurgeKind::Registry {
                continue;
            }
            let path = entry.path.as_ref().expect("file entries carry a path");
            assert!(Path::new(path).is_absolute(), "{path} is not absolute");
            assert!(!path.contains(".."), "{path} was not lexically cleaned");
        }
    }

    #[test]
    fn absolutize_cleans_without_resolving_symlinks() {
        // "/a" is not absolute on Windows — it lacks a drive prefix — so the
        // fixtures have to be written in the host's own absolute form.
        let (messy, clean, above_root) = if cfg!(windows) {
            (r"C:\a\.\b\..\c", r"C:\a\c", r"C:\..\..")
        } else {
            ("/a/./b/../c", "/a/c", "/../..")
        };
        assert_eq!(absolutize(Path::new(messy)), Some(PathBuf::from(clean)));
        assert_eq!(
            absolutize(Path::new(above_root)),
            None,
            "climbing above the root has no answer, and must not silently become the root"
        );

        // Whatever the host calls absolute, the output is: the executor runs
        // from a temp directory and cannot re-resolve a relative path.
        let rel = absolutize(Path::new("rel")).unwrap();
        assert!(rel.is_absolute());
    }

    // ── Enumeration seams (kept honest by tests that fail if they vanish) ──

    #[test]
    fn platform_artifacts_are_enumerated_from_the_probe_roots() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let receipt_dir = root.path().join("r");
        std::fs::create_dir_all(&receipt_dir).unwrap();

        // Create whichever webview location this platform probes.
        for rel in [
            "Library/WebKit/com.cloto.app",
            "Library/Caches/com.cloto.app",
            "Library/Application Support/com.cloto.app",
        ] {
            std::fs::create_dir_all(home.join(rel)).unwrap();
        }
        let platform_data = root.path().join("xdg-data");
        let platform_local = root.path().join("localappdata");
        std::fs::create_dir_all(platform_data.join("com.cloto.app")).unwrap();
        std::fs::create_dir_all(platform_local.join("com.cloto.app").join("EBWebView")).unwrap();

        let mut req = isolated(&receipt_dir, PurgeTier::Everything);
        req.roots.home = Some(home);
        req.roots.platform_data = Some(platform_data);
        req.roots.platform_local = Some(platform_local);
        let plan = build_plan(&req);

        if cfg!(any(
            target_os = "macos",
            target_os = "linux",
            target_os = "windows"
        )) {
            assert!(
                plan.entries
                    .iter()
                    .any(|e| e.id.starts_with("webview") && e.source == PurgeSource::Platform),
                "webview data is exactly the footprint no other path cleans up: {:?}",
                ids(&plan)
            );
        }
    }

    #[test]
    fn a_stray_data_directory_is_enumerated_and_never_below_tier_four() {
        let root = tempfile::tempdir().unwrap();
        let exe_dir = root.path().join("exe");
        touch(&exe_dir.join("data/cloto_memories.db"), 10);
        let receipt_dir = root.path().join("r");
        std::fs::create_dir_all(&receipt_dir).unwrap();

        let mut req = isolated(&receipt_dir, PurgeTier::Everything);
        req.roots.exe_dir = Some(exe_dir.clone());
        let plan = build_plan(&req);
        let stray = plan
            .entries
            .iter()
            .find(|e| e.source == PurgeSource::Legacy)
            .expect("a stray data dir with a database must be enumerated");
        assert_eq!(
            stray.path.as_deref(),
            Some(exe_dir.join("data").display().to_string().as_str())
        );
        assert_eq!(stray.tier, PurgeTier::Everything);

        // It may hold another installation's data, so no narrower scope
        // offers to remove it.
        let mut narrow = isolated(&receipt_dir, PurgeTier::Assets);
        narrow.roots.exe_dir = Some(exe_dir);
        let plan = build_plan(&narrow);
        assert!(plan.entries.iter().all(|e| e.source != PurgeSource::Legacy));
    }

    #[test]
    fn a_prefix_argument_is_covered_even_without_a_receipt() {
        let root = tempfile::tempdir().unwrap();
        let prefix = root.path().join("opt/cloto");
        touch(&prefix.join("clotocore"), 7);
        let empty = root.path().join("no-receipt");
        std::fs::create_dir_all(&empty).unwrap();

        let plan = build_plan(&isolated(&empty, PurgeTier::Application));
        assert!(
            plan.entries.is_empty(),
            "without --prefix there is nothing to go on"
        );

        let req = isolated(&empty, PurgeTier::Application).with_prefix(Some(prefix.clone()));
        let plan = build_plan(&req);
        assert!(
            ids(&plan).contains(&"install_prefix"),
            "the dry run must describe what the real uninstall would remove"
        );
        assert!(!plan.notes.iter().any(|n| n.contains("--prefix is given")));
    }

    // ── Measurement ──

    #[test]
    fn measure_sums_a_tree_and_ignores_missing_paths() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("a"), 10);
        touch(&dir.path().join("sub/b"), 5);
        assert_eq!(measure(dir.path()), (15, false));
        assert_eq!(measure(&dir.path().join("nope")), (0, false));
    }

    #[cfg(unix)]
    #[test]
    fn measure_does_not_follow_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        touch(&outside.path().join("big"), 10_000);
        touch(&dir.path().join("real"), 7);
        std::os::unix::fs::symlink(outside.path().join("big"), dir.path().join("link")).unwrap();

        let (bytes, truncated) = measure(dir.path());
        assert_eq!(
            bytes, 7,
            "a symlink's target lives elsewhere; counting it would promise to free bytes that \
             are not ours"
        );
        assert!(!truncated);
    }

    #[test]
    fn a_truncated_walk_says_so_all_the_way_up_to_the_total() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            touch(&dir.path().join(format!("f{i}")), 100);
        }
        let (bytes, truncated) = measure_with_limit(dir.path(), 2);
        assert!(truncated, "the walk stopped early and must admit it");
        assert!(bytes < 500);

        let plan = PurgePlan {
            plan_version: PLAN_VERSION,
            app_version: "test".into(),
            generated_at: "now".into(),
            tier: PurgeTier::Everything,
            data_dir: "/d".into(),
            entries: vec![PurgeEntry {
                id: "data_dir".into(),
                kind: PurgeKind::Dir,
                path: Some("/d".into()),
                name: None,
                tier: PurgeTier::Everything,
                source: PurgeSource::Receipt,
                size_bytes: Some(bytes),
                size_truncated: true,
                unreadable: false,
                secret: false,
                covers_secret: false,
            }],
            skipped: vec![],
            notes: vec![],
        };
        assert!(plan.total_truncated());
        assert!(
            render_text(&plan).contains(&format!("{}+ total", human_bytes(bytes))),
            "the total must carry the lower-bound marker too"
        );
    }

    // ── Ordering ──

    #[test]
    fn deregistrations_run_before_the_files_they_depend_on() {
        let entries = vec![
            PurgeEntry {
                id: "autostart".into(),
                kind: PurgeKind::File,
                path: Some("/Users/x/Library/LaunchAgents/com.cloto.system.plist".into()),
                name: None,
                tier: PurgeTier::Application,
                source: PurgeSource::Platform,
                size_bytes: Some(1),
                size_truncated: false,
                unreadable: false,
                secret: false,
                covers_secret: false,
            },
            PurgeEntry {
                id: "service".into(),
                kind: PurgeKind::Service,
                path: None,
                name: Some("com.cloto.system".into()),
                tier: PurgeTier::Application,
                source: PurgeSource::Receipt,
                size_bytes: None,
                size_truncated: false,
                unreadable: false,
                secret: false,
                covers_secret: false,
            },
        ];
        let ordered = order_for_removal(entries);
        assert_eq!(
            ordered[0].kind,
            PurgeKind::Service,
            "unloading a launchd job only works while its plist still exists"
        );
    }

    #[test]
    fn files_are_ordered_deepest_first() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("d");
        touch(&data.join("a/b/c"), 1);
        touch(&data.join("top"), 1);
        footprint::record(
            &root.path().join("r"),
            vec![
                ReceiptEntry::file("db", &data.join("a/b/c")),
                ReceiptEntry::file("seal_key", &data.join("top")),
            ],
        );
        let plan = build_plan(&isolated(&root.path().join("r"), PurgeTier::UserData));
        let depths: Vec<usize> = plan
            .entries
            .iter()
            .filter_map(|e| e.path.as_ref())
            .map(|p| Path::new(p).components().count())
            .collect();
        assert!(
            depths.windows(2).all(|w| w[0] >= w[1]),
            "children must be removed before their parents: {depths:?}"
        );
    }

    // ── Rendering and CLI ──

    #[test]
    fn rendering_states_the_tier_and_that_nothing_was_removed() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("cloto_memories.db"), 2048);
        footprint::record(
            dir.path(),
            vec![ReceiptEntry::file(
                "db",
                &dir.path().join("cloto_memories.db"),
            )],
        );

        let text = render_text(&build_plan(&isolated(dir.path(), PurgeTier::UserData)));
        assert!(text.contains("Scope tier:     2"));
        assert!(text.contains("cloto_memories.db"));
        assert!(text.contains("2.0 KB"));
        assert!(
            text.contains("[recorded]"),
            "source must be visible: {text}"
        );
        assert!(
            text.contains("nothing was removed"),
            "a plan rendering must never read as a completed uninstall"
        );
    }

    #[test]
    fn human_bytes_scales() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2.0 KB");
        assert_eq!(human_bytes(3 * 1_048_576), "3.0 MB");
        assert_eq!(human_bytes(2 * 1_073_741_824), "2.0 GB");
    }

    #[test]
    fn cli_rejects_a_tier_outside_the_documented_range() {
        assert!(run_cli(0, None, true).is_err());
        assert!(run_cli(5, None, true).is_err());
    }

    #[test]
    fn a_missing_receipt_yields_a_plan_that_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let plan = build_plan(&isolated(dir.path(), PurgeTier::Everything));
        assert!(
            plan.notes.iter().any(|n| n.contains("No install receipt")),
            "an incomplete enumeration must announce itself"
        );
    }

    #[test]
    fn plan_round_trips_through_json() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("seal.key"), 4);
        footprint::record(
            dir.path(),
            vec![
                ReceiptEntry::dir("data_dir", dir.path()),
                ReceiptEntry::file("seal_key", &dir.path().join("seal.key")).secret(),
            ],
        );
        let plan = build_plan(&isolated(dir.path(), PurgeTier::Everything));
        let json = serde_json::to_string(&plan).unwrap();
        let back: PurgePlan = serde_json::from_str(&json).unwrap();
        assert_eq!(plan, back, "the plan is the executor's only input");
    }
}
