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
    /// Found beside a path the receipt *does* record, and belonging to it: a
    /// database's `-wal` / `-shm` sidecars, the copies the app takes before a
    /// risky migration. Not "recorded" — the receipt never named it — and not
    /// a platform artifact either, so a reviewer can see that the plan reached
    /// past the ledger to get it.
    Derived,
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
            Self::Derived => "beside a recorded file",
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
    // One icon per installed size, discovered rather than enumerated, so the
    // ids are indexed. They are part of the application's desktop
    // registration, like the `.desktop` entry that points at them.
    if id.starts_with("icon_") {
        return PurgeTier::Application;
    }
    PurgeTier::UserData
}

fn classify_exact(id: &str) -> Option<PurgeTier> {
    Some(match id {
        // Tier 1 — the application itself.
        "binary" | "app_bundle" | "install_prefix" | "install_scripts" | "service"
        | "autostart" => PurgeTier::Application,
        // Desktop registration: shortcuts (Windows, per-machine and per-user)
        // and the `.desktop` entry (Linux). They belong to the application, not
        // to anything the user made.
        "start_menu_shortcut"
        | "start_menu_shortcut_user"
        | "desktop_shortcut"
        | "desktop_shortcut_user"
        | "desktop_entry" => PurgeTier::Application,
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
    /// The running binary itself. The `.deb` names its icons after it
    /// (`/usr/bin/app` → `app.png`), so the icon probe follows a renamed
    /// binary instead of a literal that would go stale.
    pub exe: Option<PathBuf>,
    /// Machine-wide application data (`%ProgramData%`). Holds the all-users
    /// Start Menu, where a per-machine install puts its shortcut.
    pub program_data: Option<PathBuf>,
    /// The public profile (`%PUBLIC%`), whose `Desktop` a per-machine install
    /// writes its desktop shortcut into — not the current user's.
    pub public: Option<PathBuf>,
    /// This user's desktop (`dirs::desktop_dir`, which follows a redirected or
    /// localised folder), where a per-user install puts its shortcut.
    pub desktop: Option<PathBuf>,
    /// System data root (`/usr/share`) for the `.desktop` entry and icons a
    /// `.deb` install lays down.
    pub system_share: Option<PathBuf>,
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
            exe: std::env::current_exe().ok(),
            program_data: std::env::var_os("ProgramData").map(PathBuf::from),
            public: std::env::var_os("PUBLIC").map(PathBuf::from),
            desktop: dirs::desktop_dir(),
            system_share: Some(PathBuf::from(SYSTEM_SHARE)),
        }
    }
}

/// Where a `.deb` install puts everything that is not the binary. Measured
/// against the shipped package, not assumed: `usr/share/applications/
/// ClotoCore.desktop` and `usr/share/icons/hicolor/<size>/apps/<binary>.png`.
const SYSTEM_SHARE: &str = "/usr/share";

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
    let (candidates, refused) = collect_candidates(req, receipt.as_ref());
    finish_plan(req, candidates, refused, receipt.as_ref())
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

/// Turn the receipt, the caller's prefix and the platform probes into
/// candidates, together with the entries refused before they could become one.
///
/// The only refusal made here is a receipt path the receipt itself could not
/// express (`ReceiptEntry::unrepresentable`). It has to happen at this seam:
/// once such a path is a candidate it is indistinguishable from a path that is
/// merely absent, and the plan would report it as already removed.
fn collect_candidates(
    req: &PlanRequest,
    receipt: Option<&Receipt>,
) -> (Vec<Candidate>, Vec<SkippedEntry>) {
    let mut out = Vec::new();
    let mut refused = Vec::new();

    if let Some(receipt) = receipt {
        for entry in &receipt.entries {
            if entry.unrepresentable {
                refused.push(SkippedEntry {
                    id: entry.id.clone(),
                    reason: SkipReason::Unsafe,
                    path: entry.path.clone(),
                });
                continue;
            }
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

            // A database is more than the file the receipt names, and its
            // sidecars are siblings rather than children — containment cannot
            // reach them, so they are derived here or not at all.
            if entry.kind == EntryKind::File && entry.id == "db" {
                if let Some(db) = entry.path.as_deref().map(Path::new) {
                    let tier = classify(&entry.id);
                    for sidecar in sqlite_sidecars(db) {
                        out.push(Candidate::path_entry(
                            sidecar_id(db, &sidecar),
                            PurgeKind::File,
                            sidecar,
                            tier,
                            PurgeSource::Derived,
                        ));
                    }
                }
            }
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
    (out, refused)
}

/// The files that live beside a SQLite database and belong to it: the `-wal`
/// and `-shm` sidecars, a rollback `-journal`, and the copies the kernel takes
/// before a risky migration (`…db.pre486bak`, `…db.corrupt-*.bak`).
///
/// Derived at plan time instead of recorded in the receipt, because the receipt
/// names the *logical* database while the set of physical files beside it is a
/// property of SQLite's journal mode and of whatever the app has done since the
/// receipt was last written. A tier-2 plan that lists the `.db` alone offers to
/// remove the user's data and then leaves the newest of it behind in a WAL that
/// was 4 MB on the machine where this was found.
///
/// The rule is a prefix match on the file name, so a sidecar or backup suffix
/// this version has never heard of is still enumerated. Anything named
/// `<database><suffix>` in the database's own directory is the database's by
/// construction.
fn sqlite_sidecars(db: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(name)) = (db.parent(), db.file_name()) else {
        return Vec::new();
    };
    // An unreadable directory yields nothing rather than an error: the
    // database entry itself is already in the plan and carries the
    // `unreadable` flag that says the enumeration could not see in there.
    let Ok(listing) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let base = name.as_encoded_bytes();
    let mut out: Vec<PathBuf> = listing
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().is_some_and(|f| {
                let bytes = f.as_encoded_bytes();
                bytes.len() > base.len() && bytes.starts_with(base)
            })
        })
        .collect();
    // Directory order is not defined; a plan has to be reproducible.
    out.sort();
    out
}

/// Plan id for a sidecar: `db` plus the part of the file name the database's
/// own name does not account for, so the plan reads `db-wal` / `db.pre486bak`
/// rather than an opaque index. Ids are labels — the path is what the executor
/// acts on — so a suffix that is not UTF-8 is rendered lossily here and refused
/// later, when the *path* is checked against what a plan file can carry.
fn sidecar_id(db: &Path, sidecar: &Path) -> String {
    let base_len = db.file_name().map_or(0, |n| n.as_encoded_bytes().len());
    let name = sidecar.file_name().map(std::ffi::OsStr::as_encoded_bytes);
    let suffix = name.map_or_else(String::new, |bytes| {
        String::from_utf8_lossy(&bytes[base_len.min(bytes.len())..]).into_owned()
    });
    format!("db{suffix}")
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
        out.extend(desktop_integration_candidates(roots));
    }

    if cfg!(target_os = "windows") {
        out.extend(identifier_container_candidates(roots));
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
        out.extend(product_registry_candidates());
        out.extend(desktop_integration_candidates(roots));
    }

    out
}

/// The per-user directories Windows names after the bundle identifier
/// (`com.cloto.app`, from `dashboard/src-tauri/tauri.conf.json`):
/// `%LOCALAPPDATA%\com.cloto.app`, which holds the webview profile
/// (`EBWebView`), and `%APPDATA%\com.cloto.app`, which is where
/// `tauri-plugin-window-state` writes `.window-state.json`.
///
/// The *containers* are the candidates, at the same granularity as macOS
/// (`Library/Application Support/com.cloto.app`) and Linux
/// (`platform_data.join("com.cloto.app")`). Windows used to name only
/// `%LOCALAPPDATA%\com.cloto.app\EBWebView`, one directory *inside* one of the
/// two, so a tier-4 uninstall reported `6 removed / 0 failed` and left both
/// identifier directories standing: the local one as an empty shell, the
/// roaming one still holding a `.window-state.json` older than the purge
/// (bug-496, measured on the Windows VM). Nothing enumerated the roaming
/// container at all.
///
/// The profile is no longer a candidate of its own: it is a strict descendant
/// of a container that is now always enumerated, so `collapse_nested` could
/// only ever report it as `covered_by_parent` — the directory that has to be
/// removed is the container.
///
/// Ids are positional over the fixed list, as on the other platforms, and the
/// index is taken before the absent roots are dropped, so `webview_0` names the
/// same location on a machine where the other probe came back empty.
///
/// Kept out of `platform_candidates` so a test on any host can assert this set:
/// that function's Windows arm is dead code everywhere else, and the Windows CI
/// job is non-blocking (`continue-on-error`), so a Windows-only assertion is
/// the weakest place to put this invariant.
fn identifier_container_candidates(roots: &ProbeRoots) -> Vec<Candidate> {
    [&roots.platform_local, &roots.platform_data]
        .into_iter()
        .enumerate()
        .filter_map(|(idx, root)| {
            root.as_ref().map(|dir| {
                Candidate::path_entry(
                    format!("webview_{idx}"),
                    PurgeKind::Dir,
                    dir.join("com.cloto.app"),
                    PurgeTier::Everything,
                    PurgeSource::Platform,
                )
            })
        })
        .collect()
}

/// The key the NSIS template writes for the installed product itself, in both
/// hives because `installMode: "both"` decides at install time which one runs.
///
/// This is not the ARP entry under `…\CurrentVersion\Uninstall` — it is the
/// separate `Software\<manufacturer>\<product>` key that Tauri's own installer
/// template creates. The purge is plan-bound and deliberately never runs the
/// NSIS uninstaller (`DEFENDER_DESIGN.md` §8.5), so anything only that
/// uninstaller knows about has to be named here or it survives: after a tier-4
/// purge whose report said `7 removed / 0 refused / 0 failed`, an independent
/// sweep of the machine still found this key (bug-497, measured on the Windows
/// VM).
///
/// The manufacturer key *above* it is intentionally left alone. `cloto` is a
/// vendor namespace rather than this product's own, so removing it would take a
/// sibling product's keys with it — `reg delete` is recursive. What remains
/// after this candidate is removed is an empty shell of a key, which is the
/// price of not deleting something that was never ours.
fn product_registry_candidates() -> Vec<Candidate> {
    ["HKLM", "HKCU"]
        .into_iter()
        .map(|hive| Candidate {
            id: format!("registry_product_{}", hive.to_lowercase()),
            kind: PurgeKind::Registry,
            path: Some(PathBuf::from(format!(
                r"{hive}\Software\{MANUFACTURER}\{PRODUCT_NAME}"
            ))),
            name: None,
            tier: PurgeTier::Everything,
            source: PurgeSource::Platform,
            secret: false,
        })
        .collect()
}

/// Product name as the installers write it: the NSIS shortcut is
/// `<PRODUCT>.lnk` and the `.deb` ships `usr/share/applications/<PRODUCT>
/// .desktop`. Mirrors `productName` in `dashboard/src-tauri/tauri.conf.json`.
const PRODUCT_NAME: &str = "ClotoCore";

/// Vendor segment of the bundle identifier (`com.cloto.app`), which is what
/// Tauri's NSIS template uses for the `Software\<manufacturer>` level. Read off
/// an installed machine (`HKLM\Software\cloto\ClotoCore`) rather than derived
/// from the template, because a name guessed wrong would make the candidate
/// report "we looked and it was not there".
const MANUFACTURER: &str = "cloto";

/// Components of the product key, below the hive. Shared with `purge_exec`,
/// whose registry floor has to admit exactly the shape this one emits — the
/// floor is what makes a plan safe to execute, so the two must not drift.
pub(crate) const PRODUCT_KEY_COMPONENTS: [&str; 3] = ["Software", MANUFACTURER, PRODUCT_NAME];

/// The Start Menu's `Programs` directory, relative to `%ProgramData%` (all
/// users) or `%APPDATA%` (one user). Shared with `purge_exec`, whose root set
/// has to admit exactly the directory this one probes.
pub(crate) const START_MENU_REL: &str = r"Microsoft\Windows\Start Menu\Programs";

/// Where the installers register the application with the desktop: Start Menu
/// and desktop shortcuts on Windows, the `.desktop` entry and its icons on
/// Linux.
///
/// Every path here was read off a real installation rather than derived from
/// documentation — a probe aimed at the wrong directory would report "we looked
/// and it was not there", which is worse than saying nothing. Measured
/// 2026-07-27 on the per-machine Windows install and in the shipped `.deb`:
///
/// ```text
/// C:\ProgramData\Microsoft\Windows\Start Menu\Programs\ClotoCore.lnk
/// C:\Users\Public\Desktop\ClotoCore.lnk
/// /usr/share/applications/ClotoCore.desktop
/// /usr/share/icons/hicolor/{32x32,128x128,256x256@2}/apps/app.png
/// ```
///
/// `installMode: "both"` means either hive can be the one that ran, so the
/// per-user locations are probed too and simply come back absent on a
/// per-machine machine. The pre-rename `cloto-system` name is deliberately
/// *not* probed: unlike the uninstall registry keys, which bug-386 showed
/// survive a rename, the legacy uninstaller removes its own shortcuts.
fn desktop_integration_candidates(roots: &ProbeRoots) -> Vec<Candidate> {
    let mut out = Vec::new();
    let shortcut = format!("{PRODUCT_NAME}.lnk");

    if cfg!(target_os = "windows") {
        let start_menu = Path::new(START_MENU_REL);
        // Per-machine first: that is where `installMode: both` landed when this
        // was measured.
        if let Some(program_data) = &roots.program_data {
            out.push(Candidate::path_entry(
                "start_menu_shortcut",
                PurgeKind::File,
                program_data.join(start_menu).join(&shortcut),
                PurgeTier::Application,
                PurgeSource::Platform,
            ));
        }
        if let Some(roaming) = &roots.platform_data {
            out.push(Candidate::path_entry(
                "start_menu_shortcut_user",
                PurgeKind::File,
                roaming.join(start_menu).join(&shortcut),
                PurgeTier::Application,
                PurgeSource::Platform,
            ));
        }
        if let Some(public) = &roots.public {
            out.push(Candidate::path_entry(
                "desktop_shortcut",
                PurgeKind::File,
                public.join("Desktop").join(&shortcut),
                PurgeTier::Application,
                PurgeSource::Platform,
            ));
        }
        if let Some(desktop) = &roots.desktop {
            out.push(Candidate::path_entry(
                "desktop_shortcut_user",
                PurgeKind::File,
                desktop.join(&shortcut),
                PurgeTier::Application,
                PurgeSource::Platform,
            ));
        }
    }

    if cfg!(target_os = "linux") {
        if let Some(share) = &roots.system_share {
            out.push(Candidate::path_entry(
                "desktop_entry",
                PurgeKind::File,
                share
                    .join("applications")
                    .join(format!("{PRODUCT_NAME}.desktop")),
                PurgeTier::Application,
                PurgeSource::Platform,
            ));
            // The icon is named after the binary (`/usr/bin/app` →
            // `app.png`), and the set of sizes is a packaging detail, so the
            // sizes are discovered rather than listed: a build that adds one
            // must not silently leave an icon behind.
            for (idx, icon) in icon_candidates(share, roots.exe.as_deref())
                .into_iter()
                .enumerate()
            {
                out.push(Candidate::path_entry(
                    format!("icon_{idx}"),
                    PurgeKind::File,
                    icon,
                    PurgeTier::Application,
                    PurgeSource::Platform,
                ));
            }
        }
    }

    out
}

/// `<share>/icons/hicolor/*/apps/<binary>.png`, one per size actually present.
///
/// The sizes are discovered, not listed: a package that adds one must not
/// silently leave an icon behind. The name comes from the running binary, which
/// is what the packaging derives it from, so a rename carries through instead of
/// stranding a literal.
fn icon_candidates(share: &Path, exe: Option<&Path>) -> Vec<PathBuf> {
    let Some(stem) = exe.and_then(Path::file_stem) else {
        return Vec::new();
    };
    let mut icon = PathBuf::from(stem);
    icon.set_extension("png");
    let Ok(listing) = std::fs::read_dir(share.join("icons").join("hicolor")) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = listing
        .flatten()
        .map(|size| size.path().join("apps").join(&icon))
        .filter(|p| p.is_file())
        .collect();
    // Directory order is not defined; a plan has to be reproducible.
    out.sort();
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
    refused: Vec<SkippedEntry>,
    receipt: Option<&Receipt>,
) -> PurgePlan {
    // `collect_candidates` refuses for one reason only — a receipt path that
    // cannot be written into a plan — so a non-empty seed *is* that case.
    let unrepresentable_in_receipt = !refused.is_empty();
    let mut skipped = refused;

    let (present, unrepresentable_live) = probe_existence(candidates, &mut skipped);
    let deduped = dedupe_by_path(present);
    let promoted = promote_containers(deduped);
    let in_tier = filter_by_tier(promoted, req.tier, &mut skipped);
    let (collapsed, covered) = collapse_nested(in_tier);
    skipped.extend(covered);

    let entries = order_for_removal(
        collapsed.into_iter().map(measure_entry).collect(),
        Some(&footprint::receipt_path(&req.data_dir)),
    );

    PurgePlan {
        plan_version: PLAN_VERSION,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        tier: req.tier,
        data_dir: req.data_dir.display().to_string(),
        entries,
        skipped,
        notes: notes(
            receipt,
            req,
            unrepresentable_in_receipt || unrepresentable_live,
        ),
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
///
/// Returns the surviving candidates and whether any was refused for not
/// surviving the plan file's encoding — the plan says so in its notes, since
/// that refusal is the one the user cannot infer from what is listed.
fn probe_existence(
    candidates: Vec<Candidate>,
    skipped: &mut Vec<SkippedEntry>,
) -> (Vec<Present>, bool) {
    let mut out = Vec::new();
    let mut unrepresentable = false;
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
                // Candidates that never went through the receipt — a
                // `--prefix` argument, a platform probe, the legacy scan —
                // arrive as live paths, so this is where the plan file's
                // encoding is decided for them. Refused *before* the stat:
                // a mangled path stats as NotFound, which would enter the
                // plan as "already absent" and exit the uninstall as success.
                if representable(&abs).is_none() {
                    unrepresentable = true;
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
    (out, unrepresentable)
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

fn notes(receipt: Option<&Receipt>, req: &PlanRequest, unrepresentable: bool) -> Vec<String> {
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
    if unrepresentable {
        notes.push(
            "A path on this system cannot be written into a plan file — plan files are UTF-8 \
             JSON, and that path is not valid UTF-8. It was left alone rather than reported as \
             removed, so it is still on disk and has to be removed by hand."
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

/// The path as a plan can carry it, or `None` when it cannot carry it at all.
///
/// A plan is UTF-8 JSON and `Path::display` is lossy, so a path that is not
/// valid UTF-8 — ordinary on Linux, where a file name is bytes — becomes a
/// U+FFFD-mangled string that names nothing on disk. The mangled string
/// round-trips to itself, so no later stage can tell it apart from a path that
/// was simply never there: the executor stats it, finds nothing, and records
/// the entry as an idempotent success while the real directory stays exactly
/// where it was. This check is the only place the difference is still visible,
/// which is why both seams that turn a path into a string go through it — the
/// receipt (`footprint::ReceiptEntry`) and the plan's own live candidates.
pub(crate) fn representable(path: &Path) -> Option<String> {
    let rendered = path.display().to_string();
    (PathBuf::from(&rendered).as_path() == path).then_some(rendered)
}

/// A path with nothing above it — `/`, `C:\`, a bare UNC share. The plan
/// refuses these outright; no uninstall footprint is ever a filesystem root.
pub(crate) fn is_filesystem_root(path: &Path) -> bool {
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

/// Deregistrations first, then deepest paths first, and whatever holds the
/// install receipt last.
///
/// Services must be deregistered *before* their unit or plist file is
/// deleted: `platform::uninstall_service` only unloads a launchd job when the
/// plist still exists, so removing the file first turns the deregistration
/// into a silent no-op and leaves the job loaded.
///
/// The receipt is the ledger this whole enumeration is built from, and
/// deepest-first would take the data directory that holds it before shallower
/// entries like the install prefix or the app bundle. If one of those then
/// fails, a second `clotocore uninstall --execute` has no receipt to read and
/// can no longer name anything only the receipt knew about — so the receipt
/// goes last. Nesting has already collapsed by this point, which is what makes
/// the reordering free: the surviving entries are disjoint, so none of them
/// depends on another being removed first.
///
/// `receipt_path` is the path of the receipt file itself; an entry ranks last
/// when removing it would take that file with it.
pub(crate) fn order_for_removal(
    mut entries: Vec<PurgeEntry>,
    receipt_path: Option<&Path>,
) -> Vec<PurgeEntry> {
    entries.sort_by(|a, b| {
        let rank = |e: &PurgeEntry| match e.kind {
            PurgeKind::Service | PurgeKind::Registry => 0,
            PurgeKind::File | PurgeKind::Dir => {
                if holds_receipt(e, receipt_path) {
                    2
                } else {
                    1
                }
            }
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

/// Would removing `entry` take the install receipt with it?
///
/// True for the receipt file itself and for every directory above it that the
/// plan lists (the data-directory container, an install prefix holding it).
/// An entry with no path — a service, a registry key — never holds a file.
fn holds_receipt(entry: &PurgeEntry, receipt_path: Option<&Path>) -> bool {
    match (receipt_path, entry.path.as_deref()) {
        // An empty string is a prefix of everything, which would rank an
        // unusable entry last for no reason; the executor refuses it anyway.
        (Some(receipt), Some(path)) if !path.is_empty() => receipt.starts_with(Path::new(path)),
        _ => false,
    }
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

pub(crate) fn tier_label(tier: PurgeTier) -> &'static str {
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

    /// A plan entry built without going through enumeration, for the ordering
    /// rules — which are about the shape of a list, not about a disk.
    fn purge_entry(id: &str, kind: PurgeKind, path: Option<&str>) -> PurgeEntry {
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

    /// An absolute path this platform accepts and UTF-8 does not.
    ///
    /// Built in memory, never on disk: APFS rejects the byte sequence with
    /// `EILSEQ`, so a fixture that touched the filesystem would only run on
    /// one third of the CI matrix. Nothing here needs it to exist — the plan
    /// refuses the path before it is ever stat'd, which is the whole point.
    #[cfg(unix)]
    fn unrepresentable_path() -> PathBuf {
        use std::os::unix::ffi::OsStrExt as _;
        PathBuf::from(std::ffi::OsStr::from_bytes(b"/opt/caf\xe9/cloto"))
    }

    #[cfg(windows)]
    fn unrepresentable_path() -> PathBuf {
        use std::os::windows::ffi::OsStringExt as _;
        // A lone high surrogate: valid UTF-16, and therefore a valid Windows
        // path, but not encodable as UTF-8.
        let mut wide: Vec<u16> = r"C:\opt\".encode_utf16().collect();
        wide.push(0xD800);
        wide.extend(r"\cloto".encode_utf16());
        PathBuf::from(std::ffi::OsString::from_wide(&wide))
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

    // ── Database sidecars ──

    /// A data dir holding a database, everything SQLite and the app keep beside
    /// it, and one file that merely lives there. Sizes are all different so a
    /// total can only come out right if the plan picked the right files.
    fn data_dir_with_sidecars() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("cloto_memories.db");
        touch(&db, 1_000);
        touch(&dir.path().join("cloto_memories.db-wal"), 4_000);
        touch(&dir.path().join("cloto_memories.db-shm"), 32);
        touch(&dir.path().join("cloto_memories.db.pre486bak"), 900);
        touch(&dir.path().join("cloto_memories.db-wal.pre486bak"), 1_900);
        touch(
            &dir.path().join("cloto_memories.db.corrupt-20260701.bak"),
            700,
        );
        // Not the database's: same directory, unrelated name.
        touch(&dir.path().join("seal.key"), 64);
        footprint::record(dir.path(), vec![ReceiptEntry::file("db", &db)]);
        (dir, db)
    }

    #[test]
    fn a_database_is_enumerated_with_its_sidecars_and_its_backup_copies() {
        let (dir, _db) = data_dir_with_sidecars();
        let plan = build_plan(&isolated(dir.path(), PurgeTier::UserData));

        // The defect this pins: a WAL holds committed data, and listing the
        // .db alone offered to remove "user data" while leaving 4 MB of it.
        for id in [
            "db",
            "db-wal",
            "db-shm",
            "db.pre486bak",
            "db-wal.pre486bak",
            "db.corrupt-20260701.bak",
        ] {
            assert!(
                ids(&plan).contains(&id),
                "{id} must be listed at the tier that claims to remove user data; got {:?}",
                ids(&plan)
            );
        }
        // Two stages have to agree for this to hold: the prefix rule excludes
        // the database itself, and path dedup would collapse it anyway. Pinned
        // here as the property; `sidecar_enumeration_is_ordered_…` is what
        // fails if only the second one is still doing the work.
        assert_eq!(
            ids(&plan).iter().filter(|id| **id == "db").count(),
            1,
            "the database must not be listed twice"
        );
        assert!(
            !ids(&plan).contains(&"seal.key"),
            "a prefix rule must not swallow unrelated files in the same directory"
        );
        assert_eq!(
            plan.total_bytes(),
            1_000 + 4_000 + 32 + 900 + 1_900 + 700,
            "the total has to account for every file the plan would remove"
        );
        for entry in plan.entries.iter().filter(|e| e.id != "db") {
            assert_eq!(
                entry.source,
                PurgeSource::Derived,
                "{} was not in the receipt, so the plan must not claim it was recorded",
                entry.id
            );
        }
    }

    #[test]
    fn a_sidecar_is_never_removed_below_the_tier_that_removes_the_database() {
        let (dir, _db) = data_dir_with_sidecars();
        let plan = build_plan(&isolated(dir.path(), PurgeTier::Application));
        assert!(
            ids(&plan).is_empty(),
            "tier 1 removes the application, not the database or anything beside it: {:?}",
            ids(&plan)
        );
        for id in ["db", "db-wal", "db.pre486bak"] {
            assert_eq!(
                skipped_for(&plan, id),
                Some(SkipReason::AboveTier),
                "{id} must be reported as out of scope, not omitted"
            );
        }
    }

    #[test]
    fn a_sidecar_collapses_into_the_data_directory_instead_of_double_counting() {
        let (dir, db) = data_dir_with_sidecars();
        footprint::record(
            dir.path(),
            vec![
                ReceiptEntry::dir("data_dir", dir.path()),
                ReceiptEntry::file("db", &db),
            ],
        );
        let plan = build_plan(&isolated(dir.path(), PurgeTier::Everything));

        assert!(ids(&plan).contains(&"data_dir"));
        for id in ["db", "db-wal", "db-shm"] {
            assert_eq!(
                skipped_for(&plan, id),
                Some(SkipReason::CoveredByParent),
                "{id} is inside the directory being removed"
            );
        }
        let dir_size = plan
            .entries
            .iter()
            .find(|e| e.id == "data_dir")
            .and_then(|e| e.size_bytes)
            .expect("the container is measured");
        assert_eq!(
            plan.total_bytes(),
            dir_size,
            "a sidecar counted both on its own and inside its parent inflates the total"
        );
    }

    #[test]
    fn a_sidecar_id_says_which_sidecar_it_is() {
        let db = Path::new("/data/cloto_memories.db");
        assert_eq!(
            sidecar_id(db, Path::new("/data/cloto_memories.db-wal")),
            "db-wal"
        );
        assert_eq!(
            sidecar_id(db, Path::new("/data/cloto_memories.db.pre486bak")),
            "db.pre486bak"
        );
        // Whichever branch of `classify` answers, a sidecar has to land in the
        // tier that removes the database — never tier 1.
        for id in ["db-wal", "db-shm", "db.pre486bak"] {
            assert_eq!(classify(id), PurgeTier::UserData);
            assert!(!PurgeTier::Application.includes(classify(id)));
        }
    }

    #[test]
    fn sidecar_enumeration_is_ordered_and_ignores_a_directory_it_cannot_read() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("cloto_memories.db");
        touch(&db, 1);
        touch(&dir.path().join("cloto_memories.db-wal"), 1);
        touch(&dir.path().join("cloto_memories.db-shm"), 1);
        let found = sqlite_sidecars(&db);
        let mut sorted = found.clone();
        sorted.sort();
        assert_eq!(found, sorted, "a plan has to be reproducible");
        assert_eq!(found.len(), 2);

        // A database whose directory does not exist yields nothing rather than
        // failing the whole enumeration.
        assert!(sqlite_sidecars(&dir.path().join("nowhere/cloto_memories.db")).is_empty());
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

    // ── Paths a plan file cannot carry ──

    #[cfg(any(unix, windows))]
    #[test]
    fn a_path_that_does_not_survive_utf8_is_not_representable() {
        let ordinary = if cfg!(windows) {
            r"C:\opt\cloto"
        } else {
            "/opt/cloto"
        };
        assert_eq!(
            representable(Path::new(ordinary)),
            Some(ordinary.to_string()),
            "an ordinary path is carried as itself"
        );

        let bad = unrepresentable_path();
        assert_eq!(
            representable(&bad),
            None,
            "a plan is UTF-8 JSON, and this path is not UTF-8"
        );

        // Why the check cannot be moved downstream: once rendered, the
        // mangled string round-trips to itself, so nothing later can tell it
        // apart from a path that was written faithfully.
        let mangled = PathBuf::from(bad.display().to_string());
        assert_ne!(mangled, bad);
        assert_eq!(representable(&mangled), Some(bad.display().to_string()));
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn a_recorded_path_the_plan_cannot_carry_is_refused_not_called_absent() {
        let bad = unrepresentable_path();
        let entry = ReceiptEntry::dir("install_prefix", &bad);
        assert!(
            entry.unrepresentable,
            "the receipt has to admit that its string is not the path"
        );

        let dir = tempfile::tempdir().unwrap();
        footprint::record(dir.path(), vec![entry]);
        let plan = build_plan(&isolated(dir.path(), PurgeTier::Everything));

        assert!(!ids(&plan).contains(&"install_prefix"), "{:?}", ids(&plan));
        assert_eq!(
            skipped_for(&plan, "install_prefix"),
            Some(SkipReason::Unsafe),
            "the mangled path stats as NotFound, so anything that probes it calls the directory \
             absent — and the uninstall then exits 0 with the directory still on disk"
        );
        assert!(
            plan.notes.iter().any(|n| n.contains("UTF-8")),
            "a refusal the user cannot infer from the listing has to be stated: {:?}",
            plan.notes
        );
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn a_live_path_the_plan_cannot_carry_is_refused_before_it_is_probed() {
        // `--prefix` arrives as a path, not through the receipt, so the
        // receipt's flag cannot cover it: the probe applies the same check to
        // the candidates it is handed. Platform probes and the legacy scan
        // reach the plan the same way.
        let dir = tempfile::tempdir().unwrap();
        let plan = build_plan(
            &isolated(dir.path(), PurgeTier::Everything).with_prefix(Some(unrepresentable_path())),
        );

        assert!(!ids(&plan).contains(&"install_prefix"), "{:?}", ids(&plan));
        assert_eq!(
            skipped_for(&plan, "install_prefix"),
            Some(SkipReason::Unsafe)
        );
        assert!(plan.notes.iter().any(|n| n.contains("UTF-8")));
    }

    #[test]
    fn an_ordinary_path_is_untouched_by_the_encoding_check() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("cloto_memories.db");
        touch(&db, 8);
        let entry = ReceiptEntry::file("db", &db);
        assert!(!entry.unrepresentable);
        footprint::record(dir.path(), vec![entry]);

        let plan = build_plan(&isolated(dir.path(), PurgeTier::UserData));
        assert!(ids(&plan).contains(&"db"));
        assert!(
            !plan.notes.iter().any(|n| n.contains("UTF-8")),
            "a plan that carries everything it enumerated must not warn that it did not"
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

    /// The two Windows identifier containers, as `identifier_container_candidates`
    /// derives them from the probe roots.
    fn windows_containers(root: &Path) -> (PathBuf, PathBuf, ProbeRoots) {
        let (local, roaming) = (root.join("Local"), root.join("Roaming"));
        let roots = ProbeRoots {
            platform_local: Some(local.clone()),
            platform_data: Some(roaming.clone()),
            ..ProbeRoots::default()
        };
        (
            local.join("com.cloto.app"),
            roaming.join("com.cloto.app"),
            roots,
        )
    }

    #[test]
    fn both_windows_identifier_containers_are_candidates() {
        // Asserted over the pure enumeration rather than through
        // `platform_candidates`, so it runs on every host: bug-496 was a
        // Windows-only omission, the dev platform is macOS, and the Windows CI
        // job is `continue-on-error`, which makes a `cfg!(windows)`-gated
        // assertion the weakest available place for this invariant.
        let root = Path::new(if cfg!(windows) { r"C:\probe" } else { "/probe" });
        let (local_container, roaming_container, roots) = windows_containers(root);

        let candidates = identifier_container_candidates(&roots);
        let listed: Vec<(&str, PathBuf)> = candidates
            .iter()
            .map(|c| {
                (
                    c.id.as_str(),
                    c.path.clone().expect("a container is named by path"),
                )
            })
            .collect();
        assert_eq!(
            listed,
            vec![
                ("webview_0", local_container.clone()),
                ("webview_1", roaming_container.clone()),
            ],
            "a tier-4 uninstall that leaves either identifier container behind is the bug this \
             enumeration exists for: %LOCALAPPDATA% held the webview profile and %APPDATA% held \
             the window state (bug-496)"
        );
        for candidate in &candidates {
            assert_eq!(candidate.kind, PurgeKind::Dir);
            assert_eq!(
                candidate.tier,
                PurgeTier::Everything,
                "same tier as the macOS and Linux containers, so no platform disagrees about \
                 when the identifier directory goes"
            );
            assert_eq!(candidate.source, PurgeSource::Platform);
        }

        // The index names a probe location, not a position in the output: on a
        // machine where `dirs::data_local_dir()` came back empty, `webview_1`
        // still means the roaming container.
        let roaming_only = ProbeRoots {
            platform_local: None,
            ..roots.clone()
        };
        let ids: Vec<String> = identifier_container_candidates(&roaming_only)
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(ids, vec!["webview_1".to_string()]);

        // That the enumeration is wired into the Windows arm at all is only
        // observable where that arm is live.
        if cfg!(target_os = "windows") {
            let emitted: Vec<PathBuf> = platform_candidates(&roots)
                .into_iter()
                .filter_map(|c| c.path)
                .collect();
            for expected in [&local_container, &roaming_container] {
                assert!(
                    emitted.contains(expected),
                    "`{}` is enumerated as a candidate but never reaches the Windows plan: {:?}",
                    expected.display(),
                    emitted
                );
            }
        }
    }

    #[test]
    fn the_executor_admits_the_windows_identifier_containers() {
        // A candidate the executor refuses is worse than an omission — the plan
        // offers to remove the directory and the uninstall reports `refused` for
        // it. `every_path_the_planner_emits_is_one_the_executor_will_accept`
        // makes that claim for the *host's* candidates only, and the Windows arm
        // is dead code on the dev platform, so the containers are checked
        // against the root set here as well. `from_probe` adds
        // `platform_local` / `platform_data` on every platform, so this runs
        // everywhere.
        let root = tempfile::tempdir().unwrap();
        let data_dir = root.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let (_, _, roots) = windows_containers(root.path());

        let allowed = crate::defender::purge_exec::PurgeRoots::from_probe(&data_dir, None, &roots);
        for candidate in identifier_container_candidates(&roots) {
            let path = candidate.path.expect("a container is named by path");
            assert!(
                allowed.covers(&path),
                "the planner emits `{}` ({}) but the executor would refuse it as outside this \
                 installation's footprint",
                path.display(),
                candidate.id
            );
        }
    }

    #[test]
    fn a_container_candidate_absorbs_the_webview_profile_inside_it() {
        // Why `%LOCALAPPDATA%\com.cloto.app\EBWebView` is no longer a candidate
        // of its own: with the container enumerated, the profile can only ever
        // be reported as `covered_by_parent`, and removing the container is what
        // bug-496 needed. Driven through `finish_plan` with hand-built
        // candidates so the host platform does not decide whether it runs.
        let root = tempfile::tempdir().unwrap();
        let (container, _, roots) = windows_containers(root.path());
        let profile = container.join("EBWebView");
        touch(&profile.join("Default/Cookies"), 64);
        let receipt_dir = root.path().join("r");
        std::fs::create_dir_all(&receipt_dir).unwrap();

        // The candidates are supplied rather than probed, so `req` only carries
        // the tier and the receipt location.
        let req = isolated(&receipt_dir, PurgeTier::Everything);
        let mut candidates = identifier_container_candidates(&roots);
        candidates.push(Candidate::path_entry(
            "webview_profile",
            PurgeKind::Dir,
            profile.clone(),
            PurgeTier::Everything,
            PurgeSource::Platform,
        ));
        let plan = finish_plan(&req, candidates, Vec::new(), None);

        let listed = plan
            .entries
            .iter()
            .find(|e| e.id == "webview_0")
            .expect("the container is what the plan removes");
        assert_eq!(
            listed.path.as_deref(),
            Some(container.display().to_string().as_str())
        );
        assert_eq!(
            skipped_for(&plan, "webview_profile"),
            Some(SkipReason::CoveredByParent),
            "the profile sits inside a directory the plan already removes: {:?}",
            plan.skipped
        );
        assert_eq!(
            plan.total_bytes(),
            64,
            "collapsing into the parent must not double-count the profile"
        );
    }

    /// A machine where every location the planner probes exists, so a test can
    /// see the whole candidate set instead of whichever subset this host has.
    fn populated_roots(root: &Path) -> ProbeRoots {
        let mk = |rel: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(&p).unwrap();
            p
        };
        let exe_dir = mk("prefix");
        let exe = exe_dir.join("app");
        std::fs::write(&exe, b"binary").unwrap();
        let share = mk("usr/share");
        std::fs::create_dir_all(share.join("applications")).unwrap();
        std::fs::write(
            share.join("applications/ClotoCore.desktop"),
            b"[Desktop Entry]",
        )
        .unwrap();
        // Two sizes present, one theme directory holding no icon at all: the
        // probe has to discover sizes rather than assume a fixed list.
        for size in ["32x32", "128x128"] {
            let apps = share.join("icons/hicolor").join(size).join("apps");
            std::fs::create_dir_all(&apps).unwrap();
            std::fs::write(apps.join("app.png"), b"icon").unwrap();
        }
        std::fs::create_dir_all(share.join("icons/hicolor/16x16/apps")).unwrap();

        let program_data = mk("ProgramData");
        let start_menu = program_data.join(START_MENU_REL);
        std::fs::create_dir_all(&start_menu).unwrap();
        std::fs::write(start_menu.join("ClotoCore.lnk"), b"lnk").unwrap();
        let public = mk("Public");
        std::fs::create_dir_all(public.join("Desktop")).unwrap();
        std::fs::write(public.join("Desktop/ClotoCore.lnk"), b"lnk").unwrap();

        ProbeRoots {
            home: Some(mk("home")),
            platform_data: Some(mk("xdg-data")),
            platform_cache: Some(mk("cache")),
            platform_local: Some(mk("localappdata")),
            exe_dir: Some(exe_dir),
            exe: Some(exe),
            program_data: Some(program_data),
            public: Some(public),
            desktop: Some(mk("desktop")),
            system_share: Some(share),
        }
    }

    #[test]
    fn the_desktop_registration_is_enumerated_at_the_application_tier() {
        let root = tempfile::tempdir().unwrap();
        let receipt_dir = root.path().join("r");
        std::fs::create_dir_all(&receipt_dir).unwrap();
        let mut req = isolated(&receipt_dir, PurgeTier::Application);
        req.roots = populated_roots(root.path());
        let plan = build_plan(&req);

        // Measured on the real installs (see `desktop_integration_candidates`):
        // Windows puts the shortcut in the all-users Start Menu and on the
        // public desktop; the .deb ships the entry and one icon per size.
        let expected: &[&str] = if cfg!(target_os = "windows") {
            &["start_menu_shortcut", "desktop_shortcut"]
        } else if cfg!(target_os = "linux") {
            &["desktop_entry", "icon_0", "icon_1"]
        } else {
            &[]
        };
        for id in expected {
            assert!(
                ids(&plan).contains(id),
                "{id} must be removed by the tier that removes the application: {:?}",
                ids(&plan)
            );
        }
        // The theme directory with no icon in it contributes nothing, so the
        // discovered set is exactly the two sizes that exist.
        if cfg!(target_os = "linux") {
            assert!(!ids(&plan).contains(&"icon_2"));
        }
        for id in [
            "start_menu_shortcut",
            "start_menu_shortcut_user",
            "desktop_shortcut",
            "desktop_shortcut_user",
            "desktop_entry",
            "icon_0",
        ] {
            assert_eq!(
                classify(id),
                PurgeTier::Application,
                "{id} is part of the application, not of anything the user made"
            );
        }
    }

    #[test]
    fn every_product_registry_key_is_one_the_executor_will_accept() {
        // The companion to the cross-check below, which can only reach the
        // registry branch on Windows: these candidates take no roots, so the
        // planner↔executor agreement they need is assertable on every host. That
        // matters because the drift this catches is what bug-497's fix had to
        // repair in two places at once — the planner started emitting a key the
        // executor's floor still refused, and a plan that names something the
        // executor will not remove is a false claim of coverage.
        let candidates = product_registry_candidates();
        assert!(
            !candidates.is_empty(),
            "an empty set would make this invariant vacuous"
        );
        for candidate in candidates {
            assert!(matches!(candidate.kind, PurgeKind::Registry));
            assert_eq!(candidate.tier, PurgeTier::Everything);
            let key = candidate
                .path
                .as_deref()
                .and_then(Path::to_str)
                .expect("a registry candidate carries its key as its path");
            assert!(
                crate::defender::purge_exec::is_removable_key(key),
                "the planner emits `{key}` ({}) but the executor's registry floor refuses it",
                candidate.id
            );
        }
    }

    #[test]
    fn the_vendor_key_above_the_product_key_is_never_a_candidate() {
        // `reg delete` is recursive and `cloto` is a vendor namespace, not this
        // product's own key: removing it would take a sibling product's keys
        // with it. Stated as a test because "we chose not to remove it" is
        // otherwise indistinguishable from "we forgot".
        for candidate in product_registry_candidates() {
            let key = candidate.path.as_deref().and_then(Path::to_str).unwrap();
            let components: Vec<&str> = key.split('\\').collect();
            assert_eq!(
                components.len(),
                4,
                "hive + Software + vendor + product, no shorter: {key}"
            );
            assert_eq!(*components.last().unwrap(), PRODUCT_NAME);
        }
        for vendor_only in [r"HKLM\Software\cloto", r"HKCU\Software\cloto"] {
            assert!(
                !crate::defender::purge_exec::is_removable_key(vendor_only),
                "{vendor_only} must not be removable even if a plan asks for it"
            );
        }
    }

    #[test]
    fn every_path_the_planner_emits_is_one_the_executor_will_accept() {
        // The planner and the executor have separate notions of what belongs to
        // this installation: the plan lists paths, and `purge_exec` refuses any
        // path no declared root covers. A listed path the executor refuses is
        // worse than an omission — the uninstall reports a failure for
        // something it offered to remove.
        //
        // Checked over the *candidates*, not the finished plan: a platform path
        // that does not exist on this machine never reaches the plan, so
        // asserting on entries would pass while saying nothing about the
        // artifact that only exists on a real install. That is how
        // `/etc/systemd/system/cloto.service` stayed listed-and-unremovable —
        // it is in every Linux plan and no root covered `/etc`.
        let root = tempfile::tempdir().unwrap();
        let data_dir = root.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let roots = populated_roots(root.path());
        let prefix = roots.exe_dir.clone();

        let allowed = crate::defender::purge_exec::PurgeRoots::from_probe(
            &data_dir,
            prefix.as_deref(),
            &roots,
        );
        let candidates: Vec<Candidate> = platform_candidates(&roots)
            .into_iter()
            .chain(legacy_candidates(&data_dir, &roots))
            .collect();
        assert!(
            !candidates.is_empty(),
            "this platform must probe for something, or the invariant is vacuous"
        );
        for candidate in candidates {
            // A registry key is removed through an OS call rather than by path,
            // so the roots do not apply — but the executor has a second floor
            // for exactly these, and skipping the kind outright is what let
            // `HKLM\Software\cloto\ClotoCore` be planned-and-refused (bug-497).
            // Same invariant, different floor.
            if matches!(candidate.kind, PurgeKind::Registry) {
                let key = candidate
                    .path
                    .as_deref()
                    .and_then(Path::to_str)
                    .expect("a registry candidate carries its key as its path");
                assert!(
                    crate::defender::purge_exec::is_removable_key(key),
                    "the planner can emit the registry key `{key}` ({}) but the executor's \
                     registry floor would refuse it",
                    candidate.id
                );
                continue;
            }
            if matches!(candidate.kind, PurgeKind::Service) {
                continue; // removed through an OS call that ignores the plan's name
            }
            let Some(path) = candidate.path.as_deref() else {
                continue;
            };
            assert!(
                allowed.covers(path),
                "the planner can emit `{}` ({}) but the executor would refuse it as outside this \
                 installation's footprint",
                path.display(),
                candidate.id
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
        let ordered = order_for_removal(entries, None);
        assert_eq!(
            ordered[0].kind,
            PurgeKind::Service,
            "unloading a launchd job only works while its plist still exists"
        );
    }

    #[test]
    fn the_entry_holding_the_receipt_is_removed_last() {
        // The convergence property: if a shallower entry fails, the next
        // `uninstall --execute` still has a receipt to rebuild its plan from.
        // The container is *deeper* than the app bundle here, so deepest-first
        // alone would put it first — the assertion fails if the ranking goes.
        let (data_dir, bundle) = if cfg!(windows) {
            (r"C:\opt\cloto\data", r"C:\Applications\ClotoCore.app")
        } else {
            ("/opt/cloto/data", "/Applications/ClotoCore.app")
        };
        let receipt = footprint::receipt_path(Path::new(data_dir));
        let receipt_str = receipt.to_string_lossy().into_owned();

        let ordered = order_for_removal(
            vec![
                purge_entry("data_dir", PurgeKind::Dir, Some(data_dir)),
                purge_entry("app_bundle", PurgeKind::Dir, Some(bundle)),
                PurgeEntry {
                    name: Some("com.cloto.system".to_string()),
                    ..purge_entry("service", PurgeKind::Service, None)
                },
            ],
            Some(&receipt),
        );

        let order: Vec<&str> = ordered.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(
            order,
            vec!["service", "app_bundle", "data_dir"],
            "deregistration first, then the rest, and the receipt's own directory last"
        );

        // The receipt file itself, listed on its own, is the same case.
        let ordered = order_for_removal(
            vec![
                purge_entry("receipt", PurgeKind::File, Some(&receipt_str)),
                purge_entry("app_bundle", PurgeKind::Dir, Some(bundle)),
            ],
            Some(&receipt),
        );
        assert_eq!(
            ordered.last().map(|e| e.id.as_str()),
            Some("receipt"),
            "the ledger goes after everything it can still describe"
        );

        // Without a receipt to protect, ordering is unchanged: deepest first.
        let ordered = order_for_removal(
            vec![
                purge_entry("data_dir", PurgeKind::Dir, Some(data_dir)),
                purge_entry("app_bundle", PurgeKind::Dir, Some(bundle)),
            ],
            None,
        );
        assert_eq!(
            ordered[0].id, "data_dir",
            "the deeper path still goes first"
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
