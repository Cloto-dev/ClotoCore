//! MGP Section 8 L0: Magic Seal — HMAC-SHA256 verification of server binary integrity.
//!
//! Provides cryptographic sealing and verification of MCP server binaries
//! to detect tampering. The seal is an HMAC-SHA256 computed over the file
//! contents using a per-installation key.

use std::path::Path;

use anyhow::{bail, Context};
use hmac::{Hmac, Mac};
use rand::Rng;
use sha2::Sha256;

use super::mcp_mgp::TrustLevel;

type HmacSha256 = Hmac<Sha256>;

/// Seal prefix used in the "sha256:{hex}" format.
const SEAL_PREFIX: &str = "sha256:";

// ============================================================
// Types
// ============================================================

/// Result of a seal verification check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SealStatus {
    /// Seal verified successfully.
    Verified,
    /// Seal verification failed (tampered or wrong key).
    Failed,
    /// No seal provided but allowed by trust level.
    Unsigned,
    /// Seal check skipped (development mode).
    Skipped,
}

// ============================================================
// Core Functions
// ============================================================

/// Compute HMAC-SHA256 of a file's contents.
/// Returns `"sha256:{hex_digest}"` format.
pub fn compute_seal(file_path: &Path, key: &[u8]) -> anyhow::Result<String> {
    let data = std::fs::read(file_path)
        .with_context(|| format!("Failed to read file for sealing: {}", file_path.display()))?;

    let mut mac =
        HmacSha256::new_from_slice(key).context("Invalid HMAC key length (should not happen)")?;
    mac.update(&data);
    let result = mac.finalize();
    let digest = hex::encode(result.into_bytes());

    Ok(format!("{SEAL_PREFIX}{digest}"))
}

/// Verify a file's HMAC-SHA256 against a stored seal value.
///
/// Returns `true` if the computed seal matches `expected_seal`, `false` otherwise.
/// Returns an error only on I/O or format problems — a mismatch is not an error.
pub fn verify_seal(file_path: &Path, expected_seal: &str, key: &[u8]) -> anyhow::Result<bool> {
    if !expected_seal.starts_with(SEAL_PREFIX) {
        bail!(
            "Invalid seal format: expected 'sha256:...' but got '{}'",
            expected_seal
        );
    }

    let computed = compute_seal(file_path, key)?;
    // Constant-time comparison of the hex strings to avoid timing side-channels.
    Ok(subtle::ConstantTimeEq::ct_eq(computed.as_bytes(), expected_seal.as_bytes()).into())
}

/// Check seal status based on trust level and configuration.
///
/// This function only computes the cryptographic outcome. Translating the
/// outcome into a startup decision (block / start under untrusted profile /
/// start under declared profile) is the caller's job — see the v0.6.3
/// behavior table below.
///
/// Behavior matrix (MGP_ISOLATION_DESIGN.md §4.0, v0.6.3+):
///
/// | Trust Level   | Seal Present | Seal Absent       | Seal Invalid |
/// |---------------|--------------|-------------------|--------------|
/// | Core          | Verify       | Force `untrusted` | Block        |
/// | Standard      | Verify       | Force `untrusted` | Block        |
/// | Experimental  | Verify       | Force `untrusted` | Block        |
/// | Untrusted     | Verify       | Allow*            | Block        |
///
/// "Force `untrusted`" means: this function returns [`SealStatus::Unsigned`];
/// the caller MUST override the effective trust_level to `Untrusted` (so the
/// isolation profile is pinned to the untrusted baseline) and emit a
/// `TRUST_LEVEL_DOWNGRADED_NO_SEAL` audit event (MGP_SECURITY.md §6.4).
///
/// * `Untrusted` + seal absent: returns [`SealStatus::Skipped`] when
///   `allow_unsigned=true` (dev mode), [`SealStatus::Unsigned`] otherwise.
///   Either way the effective trust_level is already `Untrusted`, so no
///   override is needed; the caller still allows startup.
///
/// `Seal invalid` (tampering) returns [`SealStatus::Failed`] across all tiers
/// — the caller MUST block startup. v0.6.3 only relaxed the `Seal absent` row.
pub fn check_seal(
    trust_level: &TrustLevel,
    seal_value: Option<&str>,
    entry_point: &Path,
    seal_key: &[u8],
    allow_unsigned: bool,
) -> anyhow::Result<SealStatus> {
    match seal_value {
        Some(seal) => {
            // Seal present — verify for all trust levels.
            let valid = verify_seal(entry_point, seal, seal_key)?;
            if valid {
                Ok(SealStatus::Verified)
            } else {
                // Invalid seal — block regardless of trust level.
                // For Core/Standard we also warn (caller should log at WARN level).
                Ok(SealStatus::Failed)
            }
        }
        None => {
            // Seal absent — behavior depends on trust level.
            //
            // v0.6.3: every tier returns `Unsigned` here; the caller forces the
            // effective trust_level to `Untrusted` regardless of declared tier.
            // Prior to v0.6.3 we returned `Failed` for `Untrusted` in production
            // (which blocked startup); v0.6.3 §4.0 relaxed that so an unsealed
            // `Untrusted` server starts under the same untrusted profile it
            // would have anyway. `Skipped` is reserved for dev-mode bypass.
            if matches!(trust_level, TrustLevel::Untrusted) && allow_unsigned {
                // Dev-mode bypass for already-untrusted: caller may keep the
                // declared profile. Emitted only for parity with prior versions.
                Ok(SealStatus::Skipped)
            } else {
                Ok(SealStatus::Unsigned)
            }
        }
    }
}

/// Load or generate the seal key.
///
/// Resolution order:
/// 1. `CLOTO_SEAL_KEY` environment variable (hex-encoded).
/// 2. `{data_dir}/seal.key` file (raw bytes).
/// 3. Generate a new random 32-byte key, save to `{data_dir}/seal.key`.
pub fn load_or_generate_seal_key(data_dir: &Path) -> anyhow::Result<Vec<u8>> {
    // 1. Check environment variable.
    if let Ok(env_key) = std::env::var("CLOTO_SEAL_KEY") {
        let key = hex::decode(env_key.trim())
            .context("CLOTO_SEAL_KEY environment variable is not valid hex")?;
        if key.is_empty() {
            bail!("CLOTO_SEAL_KEY environment variable is empty");
        }
        return Ok(key);
    }

    // 2. Check existing key file.
    let key_path = data_dir.join("seal.key");
    if key_path.exists() {
        let key = std::fs::read(&key_path)
            .with_context(|| format!("Failed to read seal key: {}", key_path.display()))?;
        if key.is_empty() {
            bail!("Seal key file exists but is empty: {}", key_path.display());
        }
        return Ok(key);
    }

    // 3. Generate new key.
    let mut rng = rand::thread_rng();
    let mut key = vec![0u8; 32];
    rng.fill(&mut key[..]);

    // Ensure data directory exists.
    std::fs::create_dir_all(data_dir)
        .with_context(|| format!("Failed to create data directory: {}", data_dir.display()))?;

    std::fs::write(&key_path, &key)
        .with_context(|| format!("Failed to write seal key: {}", key_path.display()))?;

    Ok(key)
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    /// Helper: create a temp file with given contents and return the path.
    fn temp_file_with(content: &[u8]) -> NamedTempFile {
        let mut f = NamedTempFile::new().expect("create temp file");
        f.write_all(content).expect("write temp file");
        f.flush().expect("flush temp file");
        f
    }

    const TEST_KEY: &[u8] = b"test-seal-key-0123456789abcdef!";

    // --------------------------------------------------------
    // compute_seal
    // --------------------------------------------------------

    #[test]
    fn compute_seal_returns_correct_format() {
        let file = temp_file_with(b"hello world");
        let seal = compute_seal(file.path(), TEST_KEY).unwrap();

        assert!(
            seal.starts_with("sha256:"),
            "seal should start with 'sha256:' but got: {seal}"
        );
        // "sha256:" = 7 chars, SHA-256 hex = 64 chars → total 71.
        assert_eq!(seal.len(), 71, "seal length mismatch: {seal}");
        // Hex portion should only contain hex characters.
        let hex_part = &seal[7..];
        assert!(
            hex_part.chars().all(|c| c.is_ascii_hexdigit()),
            "non-hex character in seal: {hex_part}"
        );
    }

    #[test]
    fn compute_seal_deterministic() {
        let file = temp_file_with(b"deterministic content");
        let seal1 = compute_seal(file.path(), TEST_KEY).unwrap();
        let seal2 = compute_seal(file.path(), TEST_KEY).unwrap();
        assert_eq!(
            seal1, seal2,
            "same file + same key should produce same seal"
        );
    }

    #[test]
    fn compute_seal_different_key_different_result() {
        let file = temp_file_with(b"same content");
        let seal_a = compute_seal(file.path(), b"key-alpha").unwrap();
        let seal_b = compute_seal(file.path(), b"key-bravo").unwrap();
        assert_ne!(
            seal_a, seal_b,
            "different keys should produce different seals"
        );
    }

    // --------------------------------------------------------
    // verify_seal
    // --------------------------------------------------------

    #[test]
    fn verify_seal_succeeds_with_correct_seal() {
        let file = temp_file_with(b"verify me");
        let seal = compute_seal(file.path(), TEST_KEY).unwrap();
        assert!(verify_seal(file.path(), &seal, TEST_KEY).unwrap());
    }

    #[test]
    fn verify_seal_fails_with_wrong_seal() {
        let file = temp_file_with(b"verify me");
        let wrong = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
        assert!(!verify_seal(file.path(), wrong, TEST_KEY).unwrap());
    }

    #[test]
    fn verify_seal_fails_with_wrong_key() {
        let file = temp_file_with(b"verify me");
        let seal = compute_seal(file.path(), TEST_KEY).unwrap();
        assert!(!verify_seal(file.path(), &seal, b"wrong-key").unwrap());
    }

    #[test]
    fn verify_seal_rejects_invalid_format() {
        let file = temp_file_with(b"whatever");
        let result = verify_seal(file.path(), "md5:abcdef", TEST_KEY);
        assert!(result.is_err(), "non-sha256 prefix should error");
    }

    // --------------------------------------------------------
    // check_seal — behavior matrix
    // --------------------------------------------------------

    /// Helper: create a sealed temp file and return (file, seal).
    fn sealed_file() -> (NamedTempFile, String) {
        let file = temp_file_with(b"sealed binary content");
        let seal = compute_seal(file.path(), TEST_KEY).unwrap();
        (file, seal)
    }

    // --- Seal Present + Valid ---

    #[test]
    fn check_seal_core_valid_seal() {
        let (file, seal) = sealed_file();
        let status =
            check_seal(&TrustLevel::Core, Some(&seal), file.path(), TEST_KEY, false).unwrap();
        assert_eq!(status, SealStatus::Verified);
    }

    #[test]
    fn check_seal_standard_valid_seal() {
        let (file, seal) = sealed_file();
        let status = check_seal(
            &TrustLevel::Standard,
            Some(&seal),
            file.path(),
            TEST_KEY,
            false,
        )
        .unwrap();
        assert_eq!(status, SealStatus::Verified);
    }

    #[test]
    fn check_seal_experimental_valid_seal() {
        let (file, seal) = sealed_file();
        let status = check_seal(
            &TrustLevel::Experimental,
            Some(&seal),
            file.path(),
            TEST_KEY,
            false,
        )
        .unwrap();
        assert_eq!(status, SealStatus::Verified);
    }

    #[test]
    fn check_seal_untrusted_valid_seal() {
        let (file, seal) = sealed_file();
        let status = check_seal(
            &TrustLevel::Untrusted,
            Some(&seal),
            file.path(),
            TEST_KEY,
            false,
        )
        .unwrap();
        assert_eq!(status, SealStatus::Verified);
    }

    // --- Seal Present + Invalid ---

    #[test]
    fn check_seal_core_invalid_seal() {
        let file = temp_file_with(b"binary");
        let bad = "sha256:badbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbad";
        let status =
            check_seal(&TrustLevel::Core, Some(bad), file.path(), TEST_KEY, false).unwrap();
        assert_eq!(status, SealStatus::Failed);
    }

    #[test]
    fn check_seal_standard_invalid_seal() {
        let file = temp_file_with(b"binary");
        let bad = "sha256:badbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbad";
        let status = check_seal(
            &TrustLevel::Standard,
            Some(bad),
            file.path(),
            TEST_KEY,
            false,
        )
        .unwrap();
        assert_eq!(status, SealStatus::Failed);
    }

    #[test]
    fn check_seal_experimental_invalid_seal() {
        let file = temp_file_with(b"binary");
        let bad = "sha256:badbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbad";
        let status = check_seal(
            &TrustLevel::Experimental,
            Some(bad),
            file.path(),
            TEST_KEY,
            false,
        )
        .unwrap();
        assert_eq!(status, SealStatus::Failed);
    }

    #[test]
    fn check_seal_untrusted_invalid_seal() {
        let file = temp_file_with(b"binary");
        let bad = "sha256:badbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbad";
        let status = check_seal(
            &TrustLevel::Untrusted,
            Some(bad),
            file.path(),
            TEST_KEY,
            false,
        )
        .unwrap();
        assert_eq!(status, SealStatus::Failed);
    }

    // --- Seal Absent ---

    #[test]
    fn check_seal_core_absent_allows() {
        let file = temp_file_with(b"binary");
        let status = check_seal(&TrustLevel::Core, None, file.path(), TEST_KEY, false).unwrap();
        assert_eq!(status, SealStatus::Unsigned);
    }

    #[test]
    fn check_seal_standard_absent_allows() {
        let file = temp_file_with(b"binary");
        let status = check_seal(&TrustLevel::Standard, None, file.path(), TEST_KEY, false).unwrap();
        assert_eq!(status, SealStatus::Unsigned);
    }

    #[test]
    fn check_seal_experimental_absent_allows() {
        let file = temp_file_with(b"binary");
        let status = check_seal(
            &TrustLevel::Experimental,
            None,
            file.path(),
            TEST_KEY,
            false,
        )
        .unwrap();
        assert_eq!(status, SealStatus::Unsigned);
    }

    #[test]
    fn check_seal_untrusted_absent_allows_v063() {
        // v0.6.3 §4.0 relaxation: an unsealed `Untrusted` server starts under
        // the same untrusted profile it would have anyway. Prior to v0.6.3
        // this returned `Failed` (Block); now it returns `Unsigned` (Allow).
        let file = temp_file_with(b"binary");
        let status =
            check_seal(&TrustLevel::Untrusted, None, file.path(), TEST_KEY, false).unwrap();
        assert_eq!(status, SealStatus::Unsigned);
    }

    // --- Untrusted + allow_unsigned ---

    #[test]
    fn check_seal_untrusted_absent_allow_unsigned_skips() {
        let file = temp_file_with(b"binary");
        let status = check_seal(&TrustLevel::Untrusted, None, file.path(), TEST_KEY, true).unwrap();
        assert_eq!(status, SealStatus::Skipped);
    }

    #[test]
    fn check_seal_untrusted_invalid_seal_allow_unsigned_still_fails() {
        let file = temp_file_with(b"binary");
        let bad = "sha256:badbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbadbad";
        let status = check_seal(
            &TrustLevel::Untrusted,
            Some(bad),
            file.path(),
            TEST_KEY,
            true,
        )
        .unwrap();
        assert_eq!(
            status,
            SealStatus::Failed,
            "allow_unsigned should not bypass invalid seal verification"
        );
    }

    // --------------------------------------------------------
    // load_or_generate_seal_key
    // --------------------------------------------------------

    #[test]
    fn load_or_generate_creates_key_file() {
        // Guard against env var pollution from parallel test (load_or_generate_reads_env_var)
        let env_guard = std::env::var("CLOTO_SEAL_KEY").ok();
        std::env::remove_var("CLOTO_SEAL_KEY");

        let dir = tempfile::tempdir().unwrap();
        let key = load_or_generate_seal_key(dir.path()).unwrap();
        assert_eq!(key.len(), 32);

        let key_path = dir.path().join("seal.key");
        assert!(key_path.exists(), "seal.key should be created");

        // Loading again should return the same key (from file, not env).
        let key2 = load_or_generate_seal_key(dir.path()).unwrap();
        assert_eq!(key, key2, "subsequent load should return same key");

        // Restore env var if it was set before
        if let Some(val) = env_guard {
            std::env::set_var("CLOTO_SEAL_KEY", val);
        }
    }

    #[test]
    fn load_or_generate_reads_env_var() {
        let dir = tempfile::tempdir().unwrap();
        let hex_key = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

        // Temporarily set the env var. This is not thread-safe but acceptable in unit tests.
        std::env::set_var("CLOTO_SEAL_KEY", hex_key);
        let key = load_or_generate_seal_key(dir.path()).unwrap();
        std::env::remove_var("CLOTO_SEAL_KEY");

        assert_eq!(key, hex::decode(hex_key).unwrap());
        // Should NOT have created a file since env var was used.
        assert!(
            !dir.path().join("seal.key").exists(),
            "seal.key should not be created when env var is set"
        );
    }
}
