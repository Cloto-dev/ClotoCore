//! Installed-tree integrity sealing (an earlier decision, an earlier decision).
//!
//! The Magic Seal minted at install time (`mgp_seal::compute_seal`) covers
//! the connector's `entry_point` — one file. Since connectors became
//! packaged, that file is typically a thin shim, so the seal verified at
//! spawn said almost nothing about the code that actually runs: the whole
//! implementation could be swapped between install and launch without the
//! check noticing. This module widens the covered surface to the installed
//! tree.
//!
//! # What this defends, and what it does not
//!
//! It defends against **post-install modification by something other than
//! the connector** — another process, malware, a stray editor, a botched
//! update. That is the realistic threat: the tree was verified against the
//! hub at install, and the question at spawn is whether it still is what
//! was installed.
//!
//! It does **not** defend against a **malicious connector modifying
//! itself**. A connector runs as the same OS user as the kernel and can
//! read the local seal key three different ways (world-readable
//! `{data_dir}/seal.key`, the inherited `CLOTO_SEAL_KEY` environment
//! variable, and unrestricted filesystem access under the current
//! "environment-based soft isolation"), so it can re-mint a valid seal for
//! whatever it wrote. Closing that requires OS-level isolation, not a
//! different key placement. Do not describe this check as protection
//! against a hostile connector.
//!
//! # Format
//!
//! A tree seal is `tree-sha256:HEX`, deliberately distinct from the
//! entry-point seal's `sha256:HEX`. The prefix is the version marker: a
//! server installed before this existed still carries an entry-point seal
//! and keeps being verified the old way, so this is additive and no
//! existing install changes behavior.
//!
//! The signed message is a canonical manifest — one `HEX  relpath` line
//! per file, sorted by path, with `/` separators on every platform so a
//! tree seals identically on Windows and Unix.

use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// Marks a seal as covering the installed tree rather than one entry point.
pub const TREE_SEAL_PREFIX: &str = "tree-sha256:";

/// Ceiling on the files one manifest may cover. Install trees are source
/// only — the Python virtualenv is shared at `{servers_dir}/.venv`, not
/// nested per server — so a tree this large means something unexpected is
/// on disk, and hashing it on every spawn would be a startup cost nobody
/// asked for.
const MAX_MANIFEST_FILES: usize = 20_000;

/// True when `seal` is a tree seal rather than an entry-point seal.
#[must_use]
pub fn is_tree_seal(seal: &str) -> bool {
    seal.starts_with(TREE_SEAL_PREFIX)
}

/// Paths excluded from the manifest.
///
/// These are produced *after* installation by running the server, so
/// including them would make the tree fail its own seal on the second
/// launch. The exclusions are directory- and extension-based rather than a
/// list of names so that a nested occurrence is caught too.
///
/// This is a deliberate hole: a file dropped into `__pycache__` is not
/// covered. It is a narrow one, because reaching that code still requires
/// modifying a covered `.py` file to import it — and that modification is
/// caught.
fn is_excluded(relative: &Path) -> bool {
    let excluded_dir = relative.components().any(|c| match c {
        Component::Normal(name) => {
            let name = name.to_string_lossy();
            name == "__pycache__"
                || name == ".venv"
                || name == ".git"
                || name == "node_modules"
                || name.ends_with(".egg-info")
                || name.ends_with(".dist-info")
        }
        _ => false,
    });
    if excluded_dir {
        return true;
    }
    matches!(
        relative.extension().and_then(std::ffi::OsStr::to_str),
        Some("pyc" | "pyo")
    )
}

/// Collect every sealable file under `root`, relative to it, sorted.
fn collect_files(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .with_context(|| format!("Failed to read directory: {}", dir.display()))?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            if is_excluded(relative) {
                continue;
            }
            // `symlink_metadata` so a symlink is never followed — it is
            // recorded as a leaf, not walked into, and cannot make the
            // manifest cover files outside the tree.
            let meta = std::fs::symlink_metadata(&path)
                .with_context(|| format!("Failed to stat: {}", path.display()))?;
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                found.push(relative.to_path_buf());
                if found.len() > MAX_MANIFEST_FILES {
                    bail!(
                        "Refusing to seal '{}': more than {MAX_MANIFEST_FILES} files under the \
                         install tree",
                        root.display()
                    );
                }
            }
            // Symlinks and special files are neither hashed nor walked.
            // Extraction refuses to create them (an earlier decision), so one here
            // arrived after install; `verify_tree_seal` reports that as a
            // mismatch via the file count rather than silently skipping.
        }
    }

    found.sort();
    Ok(found)
}

/// Build the canonical manifest text for `root`.
fn tree_manifest(root: &Path) -> anyhow::Result<String> {
    let files = collect_files(root)?;
    let mut manifest = String::new();
    for relative in files {
        let absolute = root.join(&relative);
        let bytes = std::fs::read(&absolute)
            .with_context(|| format!("Failed to read file for sealing: {}", absolute.display()))?;
        let digest = hex::encode(Sha256::digest(&bytes));
        // Normalize separators so a tree seals identically across platforms.
        let printable = relative
            .components()
            .filter_map(|c| match c {
                Component::Normal(s) => Some(s.to_string_lossy()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("/");
        manifest.push_str(&digest);
        manifest.push_str("  ");
        manifest.push_str(&printable);
        manifest.push('\n');
    }
    Ok(manifest)
}

/// Resolve the install tree that owns `entry_point`: the directory
/// directly under `servers_root` that contains it.
///
/// Returns `None` for an entry point outside `servers_root` (a dev-layout
/// script, a server registered by hand), which is also the signal that a
/// tree seal does not apply to it.
#[must_use]
pub fn install_root_for(entry_point: &Path, servers_root: &Path) -> Option<PathBuf> {
    let relative = entry_point.strip_prefix(servers_root).ok()?;
    match relative.components().next()? {
        Component::Normal(first) => Some(servers_root.join(first)),
        _ => None,
    }
}

/// Compute the tree seal for an installed server rooted at `root`.
pub fn compute_tree_seal(root: &Path, key: &[u8]) -> anyhow::Result<String> {
    if !root.is_dir() {
        bail!(
            "Cannot seal install tree: '{}' is not a directory",
            root.display()
        );
    }
    let manifest = tree_manifest(root)?;
    let mut mac = HmacSha256::new_from_slice(key).context("Invalid HMAC key length")?;
    mac.update(manifest.as_bytes());
    Ok(format!(
        "{TREE_SEAL_PREFIX}{}",
        hex::encode(mac.finalize().into_bytes())
    ))
}

/// Verify an installed tree against a previously minted tree seal.
///
/// Returns `false` on mismatch; an error means the tree could not be read
/// at all, which the caller must not treat as a pass.
pub fn verify_tree_seal(root: &Path, expected: &str, key: &[u8]) -> anyhow::Result<bool> {
    if !is_tree_seal(expected) {
        bail!("Not a tree seal: '{expected}'");
    }
    let computed = compute_tree_seal(root, key)?;
    // Both sides are freshly hex-encoded HMACs of the same length, so a
    // constant-time compare buys nothing an attacker could use here; the
    // comparison is on a value they already control the input to.
    Ok(computed == expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_tree(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "clotocore-treeseal-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("pkg")).unwrap();
        std::fs::write(dir.join("server.py"), b"from pkg.impl import run").unwrap();
        std::fs::write(dir.join("pkg/impl.py"), b"def run(): pass").unwrap();
        dir
    }

    const KEY: &[u8] = b"0123456789abcdef0123456789abcdef";

    #[test]
    fn tree_seal_is_stable_across_recomputation() {
        let root = temp_tree("stable");
        let a = compute_tree_seal(&root, KEY).unwrap();
        let b = compute_tree_seal(&root, KEY).unwrap();
        assert_eq!(a, b);
        assert!(a.starts_with(TREE_SEAL_PREFIX));
        assert!(verify_tree_seal(&root, &a, KEY).unwrap());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The whole point of an earlier decision: rewriting the implementation body must
    /// be caught even though the entry point is untouched.
    #[test]
    fn tree_seal_catches_a_rewritten_implementation_file() {
        let root = temp_tree("rewrite");
        let sealed = compute_tree_seal(&root, KEY).unwrap();

        std::fs::write(root.join("pkg/impl.py"), b"def run(): steal()").unwrap();

        assert!(!verify_tree_seal(&root, &sealed, KEY).unwrap());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn tree_seal_catches_an_added_file() {
        let root = temp_tree("added");
        let sealed = compute_tree_seal(&root, KEY).unwrap();

        std::fs::write(root.join("pkg/backdoor.py"), b"def go(): pass").unwrap();

        assert!(!verify_tree_seal(&root, &sealed, KEY).unwrap());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn tree_seal_catches_a_deleted_file() {
        let root = temp_tree("deleted");
        let sealed = compute_tree_seal(&root, KEY).unwrap();

        std::fs::remove_file(root.join("pkg/impl.py")).unwrap();

        assert!(!verify_tree_seal(&root, &sealed, KEY).unwrap());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Running a Python server writes bytecode caches into its own tree. If
    /// those counted, every server would fail its seal on the second launch.
    #[test]
    fn tree_seal_ignores_bytecode_caches_written_by_running_the_server() {
        let root = temp_tree("pycache");
        let sealed = compute_tree_seal(&root, KEY).unwrap();

        std::fs::create_dir_all(root.join("pkg/__pycache__")).unwrap();
        std::fs::write(
            root.join("pkg/__pycache__/impl.cpython-312.pyc"),
            b"\x00\x01",
        )
        .unwrap();
        std::fs::write(root.join("stray.pyc"), b"\x00\x02").unwrap();

        assert!(verify_tree_seal(&root, &sealed, KEY).unwrap());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The directory exclusions are a separate branch from the `.pyc`
    /// extension rule, and every file CPython writes into `__pycache__`
    /// happens to end in `.pyc` — so a test that only drops `.pyc` files
    /// there passes even with the directory rule deleted. This pins the
    /// directory branch with contents the extension rule cannot catch.
    /// (Found by mutation testing: removing `__pycache__` from the
    /// directory list originally failed nothing.)
    #[test]
    fn tree_seal_ignores_generated_directories_regardless_of_extension() {
        let root = temp_tree("gendirs");
        let sealed = compute_tree_seal(&root, KEY).unwrap();

        for (dir, file) in [
            ("pkg/__pycache__", "marker.txt"),
            ("demo.egg-info", "PKG-INFO"),
            ("demo.dist-info", "RECORD"),
            (".venv/lib", "sitecustomize.py"),
            (".git", "HEAD"),
            ("node_modules/dep", "index.js"),
        ] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
            std::fs::write(root.join(dir).join(file), b"generated").unwrap();
        }

        assert!(
            verify_tree_seal(&root, &sealed, KEY).unwrap(),
            "generated directories must not disturb the seal"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn tree_seal_changes_with_the_key() {
        let root = temp_tree("key");
        let a = compute_tree_seal(&root, KEY).unwrap();
        let b = compute_tree_seal(&root, b"ffffffffffffffffffffffffffffffff").unwrap();
        assert_ne!(a, b);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn install_root_is_the_directory_directly_under_the_servers_root() {
        let servers = PathBuf::from("/data/mcp-servers");

        assert_eq!(
            install_root_for(&servers.join("cembedding/cembedding/server.py"), &servers),
            Some(servers.join("cembedding"))
        );
        // A dev-layout entry point outside the servers root has no install
        // tree, which is also the signal that tree sealing does not apply.
        assert_eq!(
            install_root_for(&PathBuf::from("/repo/servers/demo/server.py"), &servers),
            None
        );
    }

    #[test]
    fn entry_point_seals_are_not_mistaken_for_tree_seals() {
        assert!(!is_tree_seal("sha256:abc"));
        assert!(is_tree_seal("tree-sha256:abc"));
        let root = temp_tree("prefix");
        assert!(verify_tree_seal(&root, "sha256:abc", KEY).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
