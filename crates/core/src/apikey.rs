//! Admin API key lifecycle: generation, persistence, and rotation.
//!
//! The admin key (`CLOTO_API_KEY`) historically had two very different
//! lifetimes: CLI installs persist it in `<prefix>/.env` (written by the
//! installer), while the desktop app generated a fresh ephemeral key on
//! every launch. This module gives both paths one persistence story so the
//! key can be handed to the user once (Setup Wizard / Settings → Security)
//! and stay valid across restarts. See
//! `docs/ONBOARDING_MODERNIZATION_DESIGN.md` §2.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Whether `CLOTO_API_KEY` was already in the process environment when the
/// entry point started, i.e. *before* any `.env` was loaded.
///
/// This has to be sampled at the entry point because it cannot be recovered
/// afterwards: `dotenvy` leaves a pre-existing variable alone, so once the
/// load has run, a key that came from the environment and a key that came
/// from the `.env` look identical. Unset means nobody sampled it (a CLI
/// subcommand, a test); that reads as `false`, which is the behaviour that
/// predates this snapshot.
static ENV_KEY_AT_BOOT: OnceLock<bool> = OnceLock::new();

/// Record whether the environment already carries the admin key. Every entry
/// point that loads a `.env` MUST call this immediately before the load.
pub fn note_env_key_before_dotenv() {
    let present = std::env::var("CLOTO_API_KEY").is_ok_and(|v| !v.trim().is_empty());
    let _ = ENV_KEY_AT_BOOT.set(present);
}

/// Whether the admin key this process runs with came from the environment
/// its parent handed it (systemd `EnvironmentFile`, a container env, a shell
/// export) rather than from a file this process controls.
#[must_use]
pub fn env_key_at_boot() -> bool {
    *ENV_KEY_AT_BOOT.get().unwrap_or(&false)
}

/// What a rotation has to say when the environment supplies the key.
///
/// `None` when the key this process booted with did not come from the
/// environment — then the file written by [`persist_key`] is also the file
/// the next boot reads, and the rotation is durable. `Some(_)` when it did:
/// the new key is live now, but the environment wins again at the next
/// start, so the write is not the whole job. Nothing else in the system can
/// notice this — the write succeeds, the process keeps serving, and only a
/// restart shows the old key coming back.
#[must_use]
pub fn rotation_persistence_note(env_key_at_boot: bool, target: &Path) -> Option<String> {
    if !env_key_at_boot {
        return None;
    }
    Some(format!(
        "This kernel received CLOTO_API_KEY from its process environment, which takes \
         precedence over {}. The new key is active now, but a restart loads the environment's \
         key again and this one stops working. Update the environment source as well (for a \
         systemd unit, the EnvironmentFile it reads).",
        target.display()
    ))
}

/// Generate a cryptographically random API key (64 hex chars).
#[must_use]
pub fn generate() -> String {
    use rand::rngs::OsRng;
    use rand::Rng;
    use std::fmt::Write;
    let bytes: [u8; 32] = OsRng.gen();
    bytes.iter().fold(String::with_capacity(64), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Resolve the `.env` file the admin key should be persisted to.
///
/// Order: `CLOTO_ENV_PATH` override → first existing of `cwd/.env`,
/// `exe_dir/.env` (CLI install layout), `data_dir/.env` (desktop layout) →
/// `data_dir/.env` as the create-if-missing default. Mirrors the load order
/// used by the entry points (`main.rs` / the Tauri boot).
#[must_use]
pub fn resolve_env_target() -> PathBuf {
    if let Ok(p) = std::env::var("CLOTO_ENV_PATH") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join(".env"));
    }
    candidates.push(crate::config::exe_dir().join(".env"));
    candidates.push(crate::config::data_dir().join(".env"));
    for c in &candidates {
        if c.exists() {
            return c.clone();
        }
    }
    crate::config::data_dir().join(".env")
}

/// Write `key` as the `CLOTO_API_KEY` line of `path`, preserving every other
/// line. Creates the file (with a short header) when missing. The write is
/// atomic (tmp + rename) and the file is restricted to the owner on Unix;
/// on Windows the desktop target (`%APPDATA%`) is already per-user scoped,
/// unlike the ProgramData case hardened in bug-463 (the installer keeps its
/// own `icacls` pass for that layout).
pub fn persist_key(path: &Path, key: &str) -> std::io::Result<()> {
    let line = format!("CLOTO_API_KEY={key}");
    let content = if path.exists() {
        let existing = std::fs::read_to_string(path)?;
        let mut replaced = false;
        let mut out: Vec<String> = existing
            .lines()
            .map(|l| {
                if l.trim_start().starts_with("CLOTO_API_KEY=") {
                    replaced = true;
                    line.clone()
                } else {
                    l.to_string()
                }
            })
            .collect();
        if !replaced {
            out.push(line);
        }
        out.join("\n") + "\n"
    } else {
        format!("# ClotoCore admin API key (auto-generated; managed by the app)\n{line}\n")
    };

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("env.tmp");
    std::fs::write(&tmp, content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)
}

/// Ensure a persistent admin key exists for this process.
///
/// If `CLOTO_API_KEY` is already set (loaded from an `.env` or the
/// environment) it is returned as-is. Otherwise a new key is generated,
/// persisted to [`resolve_env_target`], and exported to the process
/// environment. A persistence failure is downgraded to a warning — the key
/// still works for this run (the pre-existing ephemeral behavior) rather
/// than failing the boot.
pub fn ensure_persistent_key() -> String {
    if let Ok(existing) = std::env::var("CLOTO_API_KEY") {
        let trimmed = existing.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    let key = generate();
    let target = resolve_env_target();
    match persist_key(&target, &key) {
        Ok(()) => tracing::info!("🔑 Generated admin API key, persisted to {}", target.display()),
        Err(e) => tracing::warn!(
            "🔑 Generated admin API key but failed to persist to {} ({e}); key is ephemeral for this run",
            target.display()
        ),
    }
    std::env::set_var("CLOTO_API_KEY", &key);
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rotation_note_absent_when_key_is_not_from_the_environment() {
        assert!(rotation_persistence_note(false, Path::new("/etc/x/.env")).is_none());
    }

    #[test]
    fn test_rotation_note_names_the_shadowed_file_and_the_restart() {
        let note = rotation_persistence_note(true, Path::new("/var/lib/clotocore/.env"))
            .expect("a key from the environment must carry a note");
        assert!(
            note.contains("/var/lib/clotocore/.env"),
            "the note must name the file that was written: {note}"
        );
        assert!(
            note.contains("restart"),
            "the note must say what makes the new key stop working: {note}"
        );
    }

    #[test]
    fn test_generate_is_64_hex() {
        let k = generate();
        assert_eq!(k.len(), 64);
        assert!(k.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(generate(), k);
    }

    #[test]
    fn test_persist_creates_fresh_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        persist_key(&path, "abc123").unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("CLOTO_API_KEY=abc123"));
        assert!(content.starts_with('#'));
    }

    #[test]
    fn test_persist_replaces_in_place_preserving_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        std::fs::write(
            &path,
            "# header\nPORT=8081\nCLOTO_API_KEY=old\nRUST_LOG=info\n",
        )
        .unwrap();
        persist_key(&path, "newkey").unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("CLOTO_API_KEY=newkey"));
        assert!(!content.contains("old"));
        assert!(content.contains("PORT=8081"));
        assert!(content.contains("RUST_LOG=info"));
        assert!(content.contains("# header"));
    }

    #[test]
    fn test_persist_appends_when_key_line_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        std::fs::write(&path, "PORT=8081\n").unwrap();
        persist_key(&path, "appended").unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("PORT=8081"));
        assert!(content.ends_with("CLOTO_API_KEY=appended\n"));
    }
}
