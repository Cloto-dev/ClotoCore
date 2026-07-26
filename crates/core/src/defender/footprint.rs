//! Install receipt — the canonical ledger of everything ClotoCore has placed
//! on the machine (DEFENDER_DESIGN.md §3).
//!
//! The receipt is authoritative for enumeration; heuristic scanning of
//! well-known locations exists only as a fallback for installs that predate
//! it. Receipt updates are best-effort and non-fatal by design: a failed
//! receipt write must never fail the operation it records, so every mutation
//! here logs a warning instead of returning an error.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const RECEIPT_FILE: &str = "installed.json";
pub const RECEIPT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub receipt_version: u32,
    /// Version of the binary that last touched the receipt.
    pub app_version: String,
    pub installed_at: String,
    pub updated_at: String,
    pub entries: Vec<ReceiptEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    File,
    Dir,
    Service,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptEntry {
    pub id: String,
    pub kind: EntryKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// OS service name for `kind == Service` entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub secret: bool,
    /// `path` is a lossy rendering, not the path itself: what is on disk
    /// cannot be written into this ledger (see `purge::representable`). The
    /// entry is still recorded — something *is* there, and a receipt that
    /// dropped it would claim the footprint was smaller than it is — but no
    /// consumer may act on the string. The purge plan refuses it rather than
    /// probing it, because probing a mangled path reports "already gone".
    #[serde(default, skip_serializing_if = "is_false")]
    pub unrepresentable: bool,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(b: &bool) -> bool {
    !*b
}

/// Record a path as a string, saying so when the string is not the path.
///
/// Receipts and plans are both UTF-8 JSON, so one round-trip check decides for
/// both; `purge::representable` is that check, and it lives there because the
/// plan is where acting on a mangled path would delete — or fail to delete —
/// something.
fn record_path(path: &Path) -> (String, bool) {
    crate::defender::purge::representable(path)
        .map_or_else(|| (path.display().to_string(), true), |ok| (ok, false))
}

impl ReceiptEntry {
    pub fn file(id: impl Into<String>, path: &Path) -> Self {
        let (path, unrepresentable) = record_path(path);
        Self {
            id: id.into(),
            kind: EntryKind::File,
            path: Some(path),
            name: None,
            secret: false,
            unrepresentable,
        }
    }

    pub fn dir(id: impl Into<String>, path: &Path) -> Self {
        let (path, unrepresentable) = record_path(path);
        Self {
            id: id.into(),
            kind: EntryKind::Dir,
            path: Some(path),
            name: None,
            secret: false,
            unrepresentable,
        }
    }

    pub fn service(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: EntryKind::Service,
            path: None,
            name: Some(name.into()),
            secret: false,
            unrepresentable: false,
        }
    }

    #[must_use]
    pub fn secret(mut self) -> Self {
        self.secret = true;
        self
    }
}

#[must_use]
pub fn receipt_path(data_dir: &Path) -> PathBuf {
    data_dir.join(RECEIPT_FILE)
}

/// The `.app` bundle containing `exe`, if it is laid out as one
/// (`Foo.app/Contents/MacOS/foo`). The layout is macOS-only in practice, but
/// the rule is pure path arithmetic and is therefore checked on every
/// platform's test run rather than only where it matters.
#[must_use]
pub fn app_bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos_dir = exe.parent()?;
    if macos_dir.file_name()? != "MacOS" {
        return None;
    }
    let contents = macos_dir.parent()?;
    if contents.file_name()? != "Contents" {
        return None;
    }
    let bundle = contents.parent()?;
    if bundle.extension()? == "app" {
        Some(bundle.to_path_buf())
    } else {
        None
    }
}

/// Name of the uninstaller the NSIS installer writes into the install
/// directory. Its presence beside the running binary is what proves the
/// directory is an install prefix and not a build output.
const NSIS_UNINSTALLER: &str = "uninstall.exe";

/// The directory an installer laid `exe` into, if it can be shown to be one.
///
/// The evidence is `uninstall.exe` beside the binary: the NSIS installer writes
/// it into `$INSTDIR`, and nothing else does. A cargo `target/debug` has no
/// such sibling, which is what makes this safe to act on — deriving the prefix
/// from the binary's directory alone would put a developer's build output in a
/// tier-1 removal plan.
///
/// The same shape as `app_bundle_of`: a structural marker, not a guess about
/// where installs live. Pure path arithmetic plus one existence check, so it is
/// exercised on every platform's test run rather than only on Windows.
#[must_use]
pub fn install_prefix_of(exe: &Path) -> Option<PathBuf> {
    let dir = prefix_dir_of(exe)?;
    dir.join(NSIS_UNINSTALLER)
        .is_file()
        .then(|| dir.to_path_buf())
}

/// The directory that is *allowed* to be an install prefix — the binary's own,
/// unless that is a filesystem root.
///
/// Split from the marker check so the rule is decidable without a filesystem:
/// a marker at `C:\uninstall.exe` cannot be created in a test, so a check
/// folded into `install_prefix_of` would pass because the marker was missing
/// rather than because the root was refused. The purge plan refuses roots too;
/// this is the same refusal made where the claim originates, so a receipt never
/// carries `C:\` as something this installation owns.
fn prefix_dir_of(exe: &Path) -> Option<&Path> {
    let dir = exe.parent()?;
    dir.parent().is_some().then_some(dir)
}

/// Load the receipt, tolerating absence and corruption (both return `None`;
/// corruption is logged — the next `record` rewrites a valid ledger).
#[must_use]
pub fn load(data_dir: &Path) -> Option<Receipt> {
    let path = receipt_path(data_dir);
    let raw = std::fs::read_to_string(&path).ok()?;
    match serde_json::from_str::<Receipt>(&raw) {
        Ok(receipt) => Some(receipt),
        Err(e) => {
            tracing::warn!(
                "Install receipt at {} is unreadable ({e}); it will be rewritten on the next \
                 footprint mutation",
                path.display()
            );
            None
        }
    }
}

/// Upsert `entries` into the receipt (keyed by `id`), creating it on first
/// write. Best-effort: any failure is logged and swallowed.
pub fn record(data_dir: &Path, entries: Vec<ReceiptEntry>) {
    let now = chrono::Utc::now().to_rfc3339();
    let mut receipt = load(data_dir).unwrap_or_else(|| Receipt {
        receipt_version: RECEIPT_VERSION,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        installed_at: now.clone(),
        updated_at: now.clone(),
        entries: Vec::new(),
    });

    for entry in entries {
        if let Some(existing) = receipt.entries.iter_mut().find(|e| e.id == entry.id) {
            *existing = entry;
        } else {
            receipt.entries.push(entry);
        }
    }
    // The receipt is part of the footprint and lists itself (§3).
    let self_path = receipt_path(data_dir);
    if !receipt.entries.iter().any(|e| e.id == "receipt") {
        receipt
            .entries
            .push(ReceiptEntry::file("receipt", &self_path));
    }
    receipt.app_version = env!("CARGO_PKG_VERSION").to_string();
    receipt.updated_at = now;

    write_receipt(data_dir, &receipt);
}

/// Remove one entry by id. Best-effort; a missing receipt or id is a no-op.
pub fn remove(data_dir: &Path, id: &str) {
    let Some(mut receipt) = load(data_dir) else {
        return;
    };
    let before = receipt.entries.len();
    receipt.entries.retain(|e| e.id != id);
    if receipt.entries.len() == before {
        return;
    }
    receipt.updated_at = chrono::Utc::now().to_rfc3339();
    write_receipt(data_dir, &receipt);
}

fn write_receipt(data_dir: &Path, receipt: &Receipt) {
    let path = receipt_path(data_dir);
    let tmp = path.with_extension("json.tmp");
    let result = serde_json::to_string_pretty(receipt)
        .map_err(std::io::Error::other)
        .and_then(|json| std::fs::write(&tmp, json))
        .and_then(|()| std::fs::rename(&tmp, &path));
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        tracing::warn!(
            "Failed to write install receipt at {} ({e}); the operation it records is unaffected",
            path.display()
        );
    }
}

/// What the application *is* on disk, given the running binary.
///
/// Takes the executable rather than reading it, so the decisions below are
/// reachable from a test: `boot_entries` can only ever be called with the real
/// `current_exe()`, and both rules here — the macOS bundle and the Windows
/// install prefix — are exactly the ones that never fire on the machine the
/// tests run on.
fn binary_entries(exe: &Path) -> Vec<ReceiptEntry> {
    let mut entries = vec![ReceiptEntry::file("binary", exe)];
    // On macOS the desktop build runs from inside an .app bundle, so the binary
    // alone is not the application: removing it would leave a broken bundle in
    // /Applications. The uninstall plan can only remove what the receipt
    // records, so the bundle has to be recorded here.
    if let Some(bundle) = app_bundle_of(exe) {
        entries.push(ReceiptEntry::dir("app_bundle", &bundle));
    }
    // On Windows the installer lays down a directory, not a file: the binary,
    // its DLLs, its resources, and the uninstaller that removes them.
    // Recording only the binary left `C:\Program Files\ClotoCore\` and
    // `uninstall.exe` on disk after a tier-4 purge had already deleted the
    // registry keys — an install that no longer appears in Add/Remove Programs
    // and is still there.
    if let Some(prefix) = install_prefix_of(exe) {
        entries.push(ReceiptEntry::dir("install_prefix", &prefix));
    }
    entries
}

/// Standard kernel-managed footprint, refreshed on every boot. Only paths
/// that exist are recorded (plus the binary and the data dir themselves), so
/// the receipt converges toward reality instead of accumulating wishes.
#[must_use]
pub fn boot_entries(data_dir: &Path) -> Vec<ReceiptEntry> {
    let mut entries = vec![ReceiptEntry::dir("data_dir", data_dir)];

    if let Ok(exe) = std::env::current_exe() {
        entries.extend(binary_entries(&exe));
    }

    let db = crate::defender::checks::resolve_db_path(data_dir);
    if db.exists() {
        entries.push(ReceiptEntry::file("db", &db));
    }

    for (id, sub) in [
        ("attachments", "attachments"),
        ("avatars", "avatars"),
        ("vrm", "vrm"),
        ("speech", "speech"),
        ("mcp_servers_root", "mcp-servers"),
        ("mcp_sandbox", "mcp-sandbox"),
        ("bin", "bin"),
        ("logs", "logs"),
        ("tmp", "tmp"),
    ] {
        let path = data_dir.join(sub);
        if path.exists() {
            entries.push(ReceiptEntry::dir(id, &path));
        }
    }

    for (id, file) in [
        ("setup_marker", "setup-complete.json"),
        ("seal_key", "seal.key"),
    ] {
        let path = data_dir.join(file);
        if path.exists() {
            let entry = ReceiptEntry::file(id, &path);
            entries.push(if id == "seal_key" {
                entry.secret()
            } else {
                entry
            });
        }
    }

    let env_path = crate::apikey::resolve_env_target();
    if env_path.exists() {
        entries.push(ReceiptEntry::file("env", &env_path).secret());
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_creates_upserts_and_lists_itself() {
        let dir = tempfile::tempdir().unwrap();
        record(dir.path(), vec![ReceiptEntry::dir("data_dir", dir.path())]);
        let receipt = load(dir.path()).expect("receipt must exist after record");
        assert_eq!(receipt.receipt_version, RECEIPT_VERSION);
        assert!(receipt.entries.iter().any(|e| e.id == "data_dir"));
        assert!(
            receipt.entries.iter().any(|e| e.id == "receipt"),
            "receipt must list itself"
        );
        let installed_at = receipt.installed_at.clone();

        // Upsert replaces by id instead of duplicating.
        record(
            dir.path(),
            vec![ReceiptEntry::file(
                "db",
                &dir.path().join("cloto_memories.db"),
            )],
        );
        record(
            dir.path(),
            vec![ReceiptEntry::file(
                "db",
                &dir.path().join("cloto_memories.db"),
            )],
        );
        let receipt = load(dir.path()).unwrap();
        assert_eq!(
            receipt.entries.iter().filter(|e| e.id == "db").count(),
            1,
            "upsert must not duplicate entries"
        );
        assert_eq!(
            receipt.installed_at, installed_at,
            "installed_at is set once and preserved"
        );
    }

    #[test]
    fn remove_drops_entry() {
        let dir = tempfile::tempdir().unwrap();
        record(
            dir.path(),
            vec![ReceiptEntry::dir(
                "mcp:demo",
                &dir.path().join("mcp-servers/demo"),
            )],
        );
        remove(dir.path(), "mcp:demo");
        let receipt = load(dir.path()).unwrap();
        assert!(!receipt.entries.iter().any(|e| e.id == "mcp:demo"));
    }

    #[test]
    fn corrupted_receipt_is_tolerated_and_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(receipt_path(dir.path()), "not json {").unwrap();
        assert!(load(dir.path()).is_none());
        record(dir.path(), vec![ReceiptEntry::dir("data_dir", dir.path())]);
        assert!(
            load(dir.path()).is_some(),
            "record must rewrite a valid ledger"
        );
    }

    #[test]
    fn app_bundle_is_detected_from_the_bundle_layout() {
        assert_eq!(
            app_bundle_of(Path::new(
                "/Applications/ClotoCore.app/Contents/MacOS/ClotoCore"
            )),
            Some(PathBuf::from("/Applications/ClotoCore.app"))
        );
        // A plain binary is not a bundle; recording its directory would put
        // an unrelated tree (a cargo target dir, /usr/local/bin) in the
        // uninstall plan.
        assert_eq!(app_bundle_of(Path::new("/usr/local/bin/clotocore")), None);
        assert_eq!(
            app_bundle_of(Path::new("/x/ClotoCore.app/Contents/Resources/tool")),
            None
        );
        assert_eq!(
            app_bundle_of(Path::new("/x/notanapp/Contents/MacOS/x")),
            None
        );
    }

    #[test]
    fn an_install_prefix_is_recognised_only_by_the_uninstaller_beside_the_binary() {
        let dir = tempfile::tempdir().unwrap();
        let install = dir.path().join("ClotoCore");
        let exe = install.join("app.exe");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::write(&exe, b"binary").unwrap();

        // No uninstaller: this is what a cargo target directory looks like, and
        // returning its parent here would put a developer's build output — or
        // whatever else sits beside the binary — into a tier-1 removal plan.
        assert_eq!(install_prefix_of(&exe), None);

        std::fs::write(install.join("uninstall.exe"), b"uninstaller").unwrap();
        assert_eq!(install_prefix_of(&exe), Some(install.clone()));

        // A directory, not a file: a stray `uninstall.exe` *directory* is not
        // an installer's work.
        let other = dir.path().join("other");
        std::fs::create_dir_all(other.join("uninstall.exe")).unwrap();
        std::fs::write(other.join("app.exe"), b"binary").unwrap();
        assert_eq!(install_prefix_of(&other.join("app.exe")), None);
    }

    #[test]
    fn the_receipt_records_the_install_directory_and_not_merely_the_binary() {
        let dir = tempfile::tempdir().unwrap();
        let install = dir.path().join("ClotoCore");
        let exe = install.join("app.exe");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::write(&exe, b"binary").unwrap();

        let ids = |entries: &[ReceiptEntry]| -> Vec<String> {
            entries.iter().map(|e| e.id.clone()).collect()
        };

        // A bare binary: nothing to enumerate beyond it.
        assert_eq!(ids(&binary_entries(&exe)), vec!["binary"]);

        std::fs::write(install.join("uninstall.exe"), b"uninstaller").unwrap();
        let entries = binary_entries(&exe);
        assert_eq!(ids(&entries), vec!["binary", "install_prefix"]);
        let prefix = entries.iter().find(|e| e.id == "install_prefix").unwrap();
        assert_eq!(prefix.kind, EntryKind::Dir);
        assert_eq!(prefix.path.as_deref(), Some(install.to_str().unwrap()));

        // The bundle rule still fires on the layout it is for, and the two are
        // independent: a bundle has no NSIS uninstaller beside its binary.
        let bundled = Path::new("/Applications/ClotoCore.app/Contents/MacOS/ClotoCore");
        assert_eq!(ids(&binary_entries(bundled)), vec!["binary", "app_bundle"]);
    }

    #[test]
    fn a_binary_sitting_in_a_filesystem_root_has_no_prefix() {
        // A marker at `/uninstall.exe` cannot be created in a test, so this
        // asserts the path rule directly: going through `install_prefix_of`
        // here would pass because the marker was absent, which is not the same
        // as the root being refused — and would keep passing with the rule
        // deleted.
        let (root_child, nested) = if cfg!(windows) {
            (
                PathBuf::from(r"C:\app.exe"),
                PathBuf::from(r"C:\ClotoCore\app.exe"),
            )
        } else {
            (
                PathBuf::from("/app.exe"),
                PathBuf::from("/opt/clotocore/app.exe"),
            )
        };
        assert_eq!(
            prefix_dir_of(&root_child),
            None,
            "a root must never be named as this installation's prefix"
        );
        assert_eq!(prefix_dir_of(&nested), Some(nested.parent().unwrap()));
        assert_eq!(prefix_dir_of(Path::new("app.exe")), None);
    }

    #[test]
    fn the_unrepresentable_flag_defaults_to_false_and_survives_the_ledger() {
        // Additive field: a receipt written before it existed must keep
        // loading, and must not read as "this path could not be recorded".
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            receipt_path(dir.path()),
            r#"{"receipt_version":1,"app_version":"0.6.7","installed_at":"2026-07-01T00:00:00Z",
                "updated_at":"2026-07-01T00:00:00Z",
                "entries":[{"id":"db","kind":"file","path":"/opt/cloto/data/cloto_memories.db"}]}"#,
        )
        .unwrap();
        let receipt = load(dir.path()).expect("a receipt without the field must still load");
        assert!(!receipt.entries[0].unrepresentable);

        // And when it is set it has to survive the write: the purge plan reads
        // the receipt back from disk, so a flag lost in JSON is a flag that
        // never ran, and the path it guards is reported as already gone.
        let mut entry = ReceiptEntry::dir("install_prefix", Path::new("/opt/cloto"));
        entry.unrepresentable = true;
        record(dir.path(), vec![entry]);
        let receipt = load(dir.path()).unwrap();
        assert!(
            receipt
                .entries
                .iter()
                .find(|e| e.id == "install_prefix")
                .expect("the entry stays in the ledger — something is there")
                .unrepresentable
        );
    }

    #[test]
    fn secret_flag_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        record(
            dir.path(),
            vec![ReceiptEntry::file("env", &dir.path().join(".env")).secret()],
        );
        let receipt = load(dir.path()).unwrap();
        let env = receipt.entries.iter().find(|e| e.id == "env").unwrap();
        assert!(env.secret);
        assert_eq!(env.kind, EntryKind::File);
    }
}
