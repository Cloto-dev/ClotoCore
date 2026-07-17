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
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(b: &bool) -> bool {
    !*b
}

impl ReceiptEntry {
    pub fn file(id: impl Into<String>, path: &Path) -> Self {
        Self {
            id: id.into(),
            kind: EntryKind::File,
            path: Some(path.display().to_string()),
            name: None,
            secret: false,
        }
    }

    pub fn dir(id: impl Into<String>, path: &Path) -> Self {
        Self {
            id: id.into(),
            kind: EntryKind::Dir,
            path: Some(path.display().to_string()),
            name: None,
            secret: false,
        }
    }

    pub fn service(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: EntryKind::Service,
            path: None,
            name: Some(name.into()),
            secret: false,
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

/// Standard kernel-managed footprint, refreshed on every boot. Only paths
/// that exist are recorded (plus the binary and the data dir themselves), so
/// the receipt converges toward reality instead of accumulating wishes.
#[must_use]
pub fn boot_entries(data_dir: &Path) -> Vec<ReceiptEntry> {
    let mut entries = vec![ReceiptEntry::dir("data_dir", data_dir)];

    if let Ok(exe) = std::env::current_exe() {
        entries.push(ReceiptEntry::file("binary", &exe));
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
