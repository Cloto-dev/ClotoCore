//! Seal parity fixtures.
//!
//! The installer's integrity checks — the tree seal minted over an installed
//! server and the install-time verdict on a catalog entry's signature — are
//! contracts whose *bytes* matter: another implementation must produce the
//! same seal for the same tree and the same verdict for the same entry, or
//! installs verified by one side fail on the other. These tests read the
//! language-neutral fixtures under `tests/fixtures/seal/` and hold this
//! crate to the recorded answers. The recorded values came from this crate;
//! change them only when the format changes on purpose, never to make a
//! failing test pass.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use cloto_core::handlers::marketplace::{
    install_seal_verdict, RegistryEntry, SealVerdict, TamperSuspect, UnsealedReason,
};
use cloto_core::managers::tree_seal::{compute_tree_seal, tree_manifest, verify_tree_seal};
use serde::Deserialize;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/seal")
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> T {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

// ── tree seal ────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct TreeCases {
    key_hex: String,
    mint: Vec<Mint>,
    manifest: Vec<Manifest>,
    verify: Vec<Verify>,
    entry_point_seal: Vec<EntryPointSeal>,
}

#[derive(Deserialize)]
struct Mint {
    tree: String,
    expected: String,
}

#[derive(Deserialize)]
struct Manifest {
    tree: String,
    file: String,
}

#[derive(Deserialize)]
struct Verify {
    name: String,
    tree: String,
    seal: String,
    #[serde(default)]
    key_hex: Option<String>,
    expect: String,
}

#[derive(Deserialize)]
struct EntryPointSeal {
    file: String,
    expected: String,
}

fn tree_root() -> PathBuf {
    fixtures().join("tree")
}

fn tree_cases() -> TreeCases {
    read_json(&tree_root().join("cases.json"))
}

fn fixture_key(cases: &TreeCases) -> Vec<u8> {
    let text = std::fs::read_to_string(tree_root().join(&cases.key_hex)).unwrap();
    hex::decode(text.trim()).unwrap()
}

#[test]
fn tree_seal_mints_the_recorded_value_for_every_fixture_tree() {
    let cases = tree_cases();
    let key = fixture_key(&cases);
    assert!(cases.mint.len() >= 3, "fixture must cover several trees");
    for m in &cases.mint {
        let got = compute_tree_seal(&tree_root().join(&m.tree), &key).unwrap();
        assert_eq!(got, m.expected, "tree '{}'", m.tree);
    }
}

/// `base` is `clean` after the server has run once: bytecode caches,
/// packaging metadata, a nested virtualenv and node_modules. They must not
/// disturb the seal, and the fixture must actually contain them — a fixture
/// that lost its generated files would pass this for the wrong reason.
#[test]
fn generated_files_are_present_in_base_and_do_not_disturb_the_seal() {
    let cases = tree_cases();
    let seal_of = |tree: &str| {
        cases
            .mint
            .iter()
            .find(|m| m.tree == tree)
            .unwrap_or_else(|| panic!("mint case for '{tree}'"))
            .expected
            .clone()
    };
    assert_eq!(seal_of("base"), seal_of("clean"));

    let base = tree_root().join("base");
    for generated in [
        "pkg/__pycache__/impl.cpython-312.pyc",
        "pkg/__pycache__/marker.txt",
        "stray.pyc",
        "stray.pyo",
        "demo.egg-info/PKG-INFO",
        "demo.dist-info/RECORD",
        "node_modules/dep/index.js",
        ".venv/lib/sitecustomize.py",
    ] {
        assert!(
            base.join(generated).is_file(),
            "fixture lost its generated file '{generated}'"
        );
    }
    let manifest = tree_manifest(&base).unwrap();
    for excluded in [
        "__pycache__",
        ".pyc",
        ".pyo",
        "egg-info",
        "dist-info",
        "node_modules",
        ".venv",
    ] {
        assert!(
            !manifest.contains(excluded),
            "'{excluded}' leaked into the manifest:\n{manifest}"
        );
    }
}

/// The manifest is the exact byte string the HMAC covers; pinning it lets a
/// divergence be located line by line instead of showing up as an opaque
/// seal mismatch.
#[test]
fn tree_manifest_matches_the_recorded_bytes() {
    let cases = tree_cases();
    assert!(!cases.manifest.is_empty());
    for m in &cases.manifest {
        let expected = std::fs::read_to_string(tree_root().join(&m.file)).unwrap();
        let got = tree_manifest(&tree_root().join(&m.tree)).unwrap();
        assert_eq!(got, expected, "manifest of '{}'", m.tree);
    }
}

/// Manifest lines are sorted by path *components*, so `pkg/sub/data.txt`
/// sorts before `pkg-extra.txt` and `pkg.txt` even though `/` (0x2F) is a
/// larger byte than `-` (0x2D) and `.` (0x2E). An implementation that sorts
/// the joined strings produces a different manifest and a different seal;
/// the fixture carries all three names so that mistake cannot hide.
#[test]
fn manifest_order_is_by_path_component_not_by_raw_bytes() {
    let manifest = tree_manifest(&tree_root().join("base")).unwrap();
    let position = |name: &str| {
        manifest
            .lines()
            .position(|line| line.ends_with(&format!("  {name}")))
            .unwrap_or_else(|| panic!("'{name}' missing from manifest"))
    };
    assert!(position("pkg/sub/data.txt") < position("pkg-extra.txt"));
    assert!(position("pkg/sub/data.txt") < position("pkg.txt"));
    // And the raw-byte order really is the other way round — otherwise the
    // assertion above would not be testing anything.
    assert!("pkg-extra.txt" < "pkg/sub/data.txt");
    assert!("pkg.txt" < "pkg/sub/data.txt");
    // Case matters: uppercase and underscore sort before lowercase.
    assert!(position("B.py") < position("_x.py"));
    assert!(position("_x.py") < position("a.py"));
}

#[test]
fn tree_seal_verify_cases_give_the_recorded_answer() {
    let cases = tree_cases();
    let default_key = fixture_key(&cases);
    let mut seen = std::collections::BTreeSet::new();
    for v in &cases.verify {
        let key = v
            .key_hex
            .as_deref()
            .map_or_else(|| default_key.clone(), |k| hex::decode(k).unwrap());
        let result = verify_tree_seal(&tree_root().join(&v.tree), &v.seal, &key);
        match v.expect.as_str() {
            "match" => assert!(result.unwrap(), "{}", v.name),
            "mismatch" => assert!(!result.unwrap(), "{}", v.name),
            "error" => assert!(result.is_err(), "{}", v.name),
            other => panic!("unknown expectation '{other}' in '{}'", v.name),
        }
        seen.insert(v.expect.clone());
    }
    // Every outcome the verifier can produce is exercised, so a fixture edit
    // that drops a class of case is caught here rather than passing quietly.
    assert_eq!(
        seen.into_iter().collect::<Vec<_>>(),
        ["error", "match", "mismatch"]
    );
}

/// When an install directory cannot be resolved the installer falls back
/// to sealing the entry point alone (`sha256:` prefix, HMAC over the file).
#[test]
fn entry_point_seal_matches_the_recorded_value() {
    let cases = tree_cases();
    let key = fixture_key(&cases);
    assert!(!cases.entry_point_seal.is_empty());
    for e in &cases.entry_point_seal {
        let got = mgp_seal::compute_seal(&tree_root().join(&e.file), &key).unwrap();
        assert_eq!(got, e.expected, "entry-point seal of '{}'", e.file);
    }
}

// ── install-time verdict on a catalog entry ──────────────────────────

#[derive(Deserialize)]
struct CatalogCases {
    jwks: String,
    cases: Vec<CatalogCase>,
}

#[derive(Deserialize)]
struct CatalogCase {
    name: String,
    entry: RegistryEntry,
    installed_entry_point_sha256: String,
    hub_key: Option<String>,
    expect: Expect,
}

#[derive(Deserialize)]
struct Expect {
    verdict: String,
    #[serde(default)]
    reason: Option<UnsealedReason>,
    #[serde(default)]
    code: Option<String>,
}

#[derive(Deserialize)]
struct Jwks {
    keys: Vec<serde_json::Value>,
}

fn catalog_root() -> PathBuf {
    fixtures().join("catalog")
}

/// Resolve `kid` the way the installer does: from the hub's JWKS document.
fn hub_key(jwks: &Jwks, kid: &str) -> mgp_seal::ed25519::PublicKey {
    jwks.keys
        .iter()
        .filter_map(|jwk| mgp_seal::ed25519::public_key_from_jwk(jwk).ok())
        .find(|(_, id)| id.as_str() == kid)
        .map_or_else(|| panic!("kid '{kid}' not in fixture JWKS"), |(pk, _)| pk)
}

#[test]
fn install_seal_verdict_agrees_with_every_recorded_case() {
    let cases: CatalogCases = read_json(&catalog_root().join("cases.json"));
    let jwks: Jwks = read_json(&catalog_root().join(&cases.jwks));
    let mut verdicts_seen = std::collections::BTreeSet::new();
    let mut verified_v1 = false;
    let mut verified_v2 = false;

    for c in &cases.cases {
        let key = c.hub_key.as_deref().map(|kid| hub_key(&jwks, kid));
        let hash_was_read = Cell::new(false);
        let installed = || {
            hash_was_read.set(true);
            Ok(c.installed_entry_point_sha256.clone())
        };
        let result = install_seal_verdict(&c.entry, installed, key.as_ref());

        match c.expect.verdict.as_str() {
            "verified" => {
                assert_eq!(result.unwrap(), SealVerdict::Verified, "{}", c.name);
                let has_archive = c
                    .entry
                    .signature_payload
                    .as_ref()
                    .is_some_and(|p| p.get("archive").is_some());
                if has_archive {
                    verified_v2 = true;
                } else {
                    verified_v1 = true;
                }
            }
            "unsealed" => {
                let reason = c.expect.reason.expect("unsealed case names a reason");
                assert_eq!(result.unwrap(), SealVerdict::Unsealed(reason), "{}", c.name);
            }
            "tamper" => {
                let code = c.expect.code.as_deref().expect("tamper case names a code");
                let err = result.expect_err(&c.name);
                let suspect = err
                    .downcast_ref::<TamperSuspect>()
                    .unwrap_or_else(|| panic!("{}: not a tamper error: {err:#}", c.name));
                assert_eq!(suspect.code, code, "{}", c.name);
            }
            other => panic!("unknown verdict '{other}' in '{}'", c.name),
        }

        // An entry without a catalog hash must be decided without touching
        // the installed file; the fixture marks those cases with a hash that
        // could never match, so reading it would have produced a tamper
        // error instead of the recorded verdict.
        if c.installed_entry_point_sha256 == "never-read" {
            assert!(!hash_was_read.get(), "{}: entry point was read", c.name);
        }
        verdicts_seen.insert(c.expect.verdict.clone());
    }

    assert_eq!(
        verdicts_seen.into_iter().collect::<Vec<_>>(),
        ["tamper", "unsealed", "verified"]
    );
    assert!(
        verified_v1,
        "fixture must carry a verified v1 (entry point only) entry"
    );
    assert!(
        verified_v2,
        "fixture must carry a verified v2 (archive-bound) entry"
    );
}

/// The recorded live entries are the point of the fixture: real hub
/// signatures under the real hub key, not keys minted by the test.
#[test]
fn catalog_fixture_uses_the_hub_signing_key() {
    let cases: CatalogCases = read_json(&catalog_root().join("cases.json"));
    let jwks: Jwks = read_json(&catalog_root().join(&cases.jwks));
    assert!(!jwks.keys.is_empty());
    let named = cases
        .cases
        .iter()
        .filter_map(|c| c.hub_key.as_deref())
        .collect::<std::collections::BTreeSet<_>>();
    for kid in named {
        hub_key(&jwks, kid);
    }
}
