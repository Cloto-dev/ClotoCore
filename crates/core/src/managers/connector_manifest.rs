//! What a connector says it is, read from the connector's own manifest.
//!
//! A connector ships `cloto-connector.json` declaring a `spec_version` and a
//! `connector_type`. Today exactly one type exists — `mgp_server`, an MCP/MGP
//! server the kernel spawns and speaks to — and that is why this module is
//! being written now rather than when the second one arrives.
//!
//! # Why read the manifest and not the catalog
//!
//! The catalog is a derived view and it is lossy: the shape the hub serves
//! (`mgp_sdk::shape::RegistryEntry`) has no `connector_type` field, so the
//! declaration is dropped exactly at the boundary the kernel would read it
//! from. The manifest is where the connector states what it is, so that is
//! what the kernel asks. A copy travelling through two hops would be a second
//! answer to a settled question, and the first thing to go stale.
//!
//! # Why refuse rather than downgrade
//!
//! An unknown *trust level* degrades to untrusted, because the kernel still
//! knows how to run the thing and is only declining to extend privilege. An
//! unknown *type* is not that: the kernel does not know how to launch it,
//! how to talk to it, or which gate applies to its calls. "Run it with fewer
//! permissions" is not a safe reading of "I do not know what this is" —
//! fewer permissions still means running it.
//!
//! # Why the refusal names a version
//!
//! A kernel that refuses a type it has never heard of is almost always an
//! old kernel meeting a new connector. If it says only "unknown", the person
//! holding it has to guess. The ability to say *which* version is needed has
//! to exist before the first new type ships, or the day it ships is the day
//! every older installation starts failing silently.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Connector types this kernel can actually run.
///
/// Adding a value here is a claim that the kernel has a launch path, a
/// transport, and an enforcement path for it — not just a name it recognises.
pub const KNOWN_CONNECTOR_TYPES: &[&str] = &["mgp_server"];

/// What a connector is assumed to be when it does not say.
///
/// Every connector predating the manifest is one of these, and 10 of the 17
/// in the registry still ship no manifest at all. Absent has to mean the one
/// type that existed when they were written, or reading the declaration would
/// be a breaking change dressed as a check.
pub const DEFAULT_CONNECTOR_TYPE: &str = "mgp_server";

/// Highest manifest `spec_version` this kernel knows how to read.
///
/// Same rule as the type, one level up: a manifest written to a newer shape
/// may put meaning in fields this kernel does not look at, and a check that
/// ignored the version would pass such a manifest by reading only the half it
/// happens to understand.
pub const MAX_SPEC_VERSION: u32 = 1;

/// The connector manifest, reduced to the two fields this check reads.
///
/// Deliberately not the whole manifest. The kernel does not own that shape —
/// the hub and the SDK do — and a struct here that mirrored all of it would
/// have to be kept in step with a repository this one does not build.
#[derive(Debug, Deserialize)]
struct Manifest {
    #[serde(default = "default_spec_version")]
    spec_version: u32,
    #[serde(default)]
    connector_type: Option<String>,
}

fn default_spec_version() -> u32 {
    1
}

/// What a connector declared, or the defaults when it declared nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub spec_version: u32,
    pub connector_type: String,
    /// True when this came from a manifest on disk rather than from the
    /// defaults. Kept so a caller can tell "said mgp_server" from "said
    /// nothing" — they are the same decision today and may not stay so.
    pub declared: bool,
}

impl Default for Declaration {
    fn default() -> Self {
        Self {
            spec_version: 1,
            connector_type: DEFAULT_CONNECTOR_TYPE.to_string(),
            declared: false,
        }
    }
}

/// Where a vendored connector's manifest can be, in the layouts that exist.
///
/// Two are in use on disk at once: the flat one written by earlier installs
/// (`<root>/<id>/`) and the nested one the current installer produces, which
/// preserves the source tree's own path (`<root>/<id>/servers/<id>/`). A
/// reader that knew only the current layout would report "no manifest" for
/// every connector installed before it changed.
fn manifest_paths(servers_root: &Path, server_id: &str) -> Vec<PathBuf> {
    let base = servers_root.join(server_id);
    vec![
        base.join("cloto-connector.json"),
        base.join("servers")
            .join(server_id)
            .join("cloto-connector.json"),
    ]
}

/// Read what `server_id` declares about itself.
///
/// A missing, unreadable or unparseable manifest yields the defaults rather
/// than an error. None of those mean "this is something strange" — they mean
/// the connector predates the manifest, or is a local checkout, or is served
/// over HTTP and was never vendored at all. The check that refuses is about
/// what a connector *says*, and these say nothing.
#[must_use]
pub fn read_declaration(servers_root: &Path, server_id: &str) -> Declaration {
    for path in manifest_paths(servers_root, server_id) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        match serde_json::from_str::<Manifest>(&text) {
            Ok(manifest) => {
                return Declaration {
                    spec_version: manifest.spec_version,
                    connector_type: manifest
                        .connector_type
                        .filter(|t| !t.trim().is_empty())
                        .unwrap_or_else(|| DEFAULT_CONNECTOR_TYPE.to_string()),
                    declared: true,
                };
            }
            Err(e) => {
                // Read but not understood. Still the defaults: a malformed
                // manifest is a packaging bug, and refusing to start over it
                // would take down connectors that work.
                tracing::warn!(
                    server = %server_id,
                    path = %path.display(),
                    error = %e,
                    "connector manifest did not parse — treating it as undeclared"
                );
                return Declaration::default();
            }
        }
    }
    Declaration::default()
}

/// `Ok` when this kernel can run what the connector says it is.
///
/// The error is the message an operator sees, so it names the version they
/// have — the question behind "unknown type" is nearly always "how old is
/// this kernel".
pub fn check_supported(declaration: &Declaration) -> Result<(), String> {
    let kernel_version = env!("CARGO_PKG_VERSION");

    if declaration.spec_version > MAX_SPEC_VERSION {
        return Err(format!(
            "connector manifest is spec_version {} and this ClotoCore reads up to {} \
             — a newer ClotoCore is required (this one is {kernel_version})",
            declaration.spec_version, MAX_SPEC_VERSION
        ));
    }

    if !KNOWN_CONNECTOR_TYPES.contains(&declaration.connector_type.as_str()) {
        return Err(format!(
            "connector declares type '{}', which this ClotoCore does not know how to run \
             (it knows: {}) — a newer ClotoCore is required (this one is {kernel_version})",
            declaration.connector_type,
            KNOWN_CONNECTOR_TYPES.join(", ")
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_manifest(dir: &Path, body: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("cloto-connector.json"), body).unwrap();
    }

    // ── what a connector says ──

    #[test]
    fn a_connector_that_says_nothing_is_the_type_that_existed_before_manifests() {
        let root = tempfile::tempdir().unwrap();
        let d = read_declaration(root.path(), "no-such-server");
        assert_eq!(d.connector_type, "mgp_server");
        assert!(!d.declared, "nothing on disk said so");
        assert!(check_supported(&d).is_ok());
    }

    #[test]
    fn the_flat_layout_is_read() {
        let root = tempfile::tempdir().unwrap();
        write_manifest(
            &root.path().join("cpersona"),
            r#"{"spec_version":1,"connector_type":"mgp_server"}"#,
        );
        let d = read_declaration(root.path(), "cpersona");
        assert!(d.declared);
        assert_eq!(d.connector_type, "mgp_server");
    }

    /// Both layouts exist on disk at once — the flat one from earlier installs
    /// and the nested one the current installer writes. Reading only the
    /// current one would report every older connector as undeclared.
    #[test]
    fn the_nested_layout_is_read_too() {
        let root = tempfile::tempdir().unwrap();
        write_manifest(
            &root
                .path()
                .join("deepseek")
                .join("servers")
                .join("deepseek"),
            r#"{"spec_version":1,"connector_type":"mgp_server"}"#,
        );
        let d = read_declaration(root.path(), "deepseek");
        assert!(d.declared);
        assert_eq!(d.connector_type, "mgp_server");
    }

    #[test]
    fn a_manifest_with_no_type_field_is_the_default_type() {
        let root = tempfile::tempdir().unwrap();
        write_manifest(&root.path().join("srv"), r#"{"spec_version":1,"id":"srv"}"#);
        let d = read_declaration(root.path(), "srv");
        assert_eq!(d.connector_type, "mgp_server");
        assert!(check_supported(&d).is_ok());
    }

    /// A packaging bug, not a strange connector. Refusing to start over it
    /// would take down something that works.
    #[test]
    fn a_manifest_that_does_not_parse_does_not_stop_the_connector() {
        let root = tempfile::tempdir().unwrap();
        write_manifest(&root.path().join("srv"), "{ this is not json");
        let d = read_declaration(root.path(), "srv");
        assert_eq!(d.connector_type, "mgp_server");
        assert!(check_supported(&d).is_ok());
    }

    // ── what the kernel will and will not run ──

    /// The whole point. A type with no launch path, no transport and no gate
    /// is not run with fewer permissions — it is not run.
    #[test]
    fn an_unknown_type_is_refused() {
        let root = tempfile::tempdir().unwrap();
        write_manifest(
            &root.path().join("dash"),
            r#"{"spec_version":1,"connector_type":"ui_module"}"#,
        );
        let d = read_declaration(root.path(), "dash");
        assert_eq!(d.connector_type, "ui_module");
        let err = check_supported(&d).unwrap_err();
        assert!(
            err.contains("ui_module"),
            "the refusal must name the type: {err}"
        );
    }

    /// The refusal has to answer "how old is this kernel", because that is the
    /// question behind almost every unknown type.
    #[test]
    fn the_refusal_names_the_version_the_operator_has() {
        let d = Declaration {
            connector_type: "ui_module".into(),
            ..Declaration::default()
        };
        let err = check_supported(&d).unwrap_err();
        assert!(err.contains(env!("CARGO_PKG_VERSION")), "got: {err}");
        assert!(err.contains("newer ClotoCore"), "got: {err}");
    }

    #[test]
    fn a_newer_manifest_shape_is_refused_as_well() {
        let d = Declaration {
            spec_version: MAX_SPEC_VERSION + 1,
            ..Declaration::default()
        };
        let err = check_supported(&d).unwrap_err();
        assert!(err.contains("spec_version"), "got: {err}");
        assert!(err.contains(env!("CARGO_PKG_VERSION")), "got: {err}");
    }

    /// The one type that exists must pass, or the check is a switch that only
    /// ever says no.
    #[test]
    fn the_type_that_exists_today_is_accepted() {
        assert!(check_supported(&Declaration::default()).is_ok());
        assert!(KNOWN_CONNECTOR_TYPES.contains(&DEFAULT_CONNECTOR_TYPE));
    }
}
