//! UI modules loaded at runtime from the data directory.
//!
//! The dashboard itself is compiled into the binary (`handlers::assets` embeds
//! `dashboard/dist/` with `rust_embed`), so today adding a screen costs a kernel
//! rebuild and a release. A module is a directory the operator drops into
//! `<data_dir>/modules/<id>/`, described by a `module.json` beside its entry
//! point. The kernel lists what is there and serves the files; it never executes
//! them and never reads them as configuration.
//!
//! Two decisions worth stating, because both could reasonably have gone the
//! other way:
//!
//! * **The routes live under `/api`.** That puts them behind
//!   `middleware::auth_middleware` — the admin key or an operator session — with
//!   no second authentication concept to keep in sync. The unauthenticated SPA
//!   shell served by `handlers::assets` is not the precedent to copy here: that
//!   shell is a fixed artifact built from this repository, whereas a module is
//!   third-party code someone placed on the host.
//! * **The directory name is the module id.** A manifest that names itself could
//!   claim an id another directory already uses, and the file system has no way
//!   to reject the duplicate. The `id` field in `module.json` is therefore read
//!   only to be checked against the directory it was found in.
//!
//! A directory whose manifest is missing or unreadable is reported as an invalid
//! entry rather than dropped from the listing. An operator who copies a module
//! in and sees nothing has no way to tell "not found" from "found and rejected",
//! and the second is the case that needs a message.

use std::path::{Component, Path as StdPath, PathBuf};
use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

use super::{check_auth, ok_data};
use crate::{AppError, AppResult, AppState};

/// Directory under `data_dir()` that holds runtime-loaded modules.
const MODULES_DIR: &str = "modules";

/// Manifest file each module directory must contain.
const MANIFEST_NAME: &str = "module.json";

fn default_entry() -> String {
    "index.html".to_string()
}

/// A module's `module.json`, as written by whoever built the module.
///
/// Only `name` is required. Everything else has a defensible default, so the
/// smallest working manifest is `{"name": "..."}` — the id comes from the
/// directory, and the entry point is conventional.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleManifest {
    /// Optional self-declaration. When present it must equal the directory
    /// name; the directory is what actually identifies the module.
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
    /// Path of the entry document, relative to the module directory.
    #[serde(default = "default_entry")]
    pub entry: String,
    /// Icon hint for the host UI. Free-form: the host decides what it can honour.
    #[serde(default)]
    pub icon: Option<String>,
    /// Kernel API paths the module asks the host to call on its behalf.
    ///
    /// Recorded here and returned in the listing; it is not enforced by these
    /// routes. Enforcement belongs to whatever hosts the module, because that
    /// is the layer holding the operator's credential — a module that is
    /// isolated from it cannot call anything regardless of what it asks for.
    #[serde(default)]
    pub requires: Vec<String>,
}

/// One row of the listing: either a module the kernel could read, or a
/// directory it found and rejected.
#[derive(Debug, Clone, Serialize)]
pub struct ModuleEntry {
    pub id: String,
    #[serde(flatten)]
    pub manifest: Option<ModuleManifest>,
    /// Why this directory is not usable. `None` for a valid module.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Reject anything that cannot be a single, safe path segment.
///
/// Kept deliberately narrower than the file system allows: a module id ends up
/// in a URL and in a path join, and the set below is what survives both without
/// encoding rules of its own.
fn is_valid_module_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Root of the module tree. Absent until an operator creates it, which is not
/// an error — it is the normal state of an installation with no modules.
fn modules_root() -> PathBuf {
    crate::config::data_dir().join(MODULES_DIR)
}

fn read_manifest(dir: &StdPath, id: &str) -> Result<ModuleManifest, String> {
    let manifest_path = dir.join(MANIFEST_NAME);
    let raw = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("cannot read {MANIFEST_NAME}: {e}"))?;
    let manifest: ModuleManifest =
        serde_json::from_str(&raw).map_err(|e| format!("invalid {MANIFEST_NAME}: {e}"))?;

    if let Some(declared) = &manifest.id {
        if declared != id {
            return Err(format!(
                "manifest id {declared:?} does not match directory {id:?}"
            ));
        }
    }
    if manifest.name.trim().is_empty() {
        return Err("manifest has an empty name".to_string());
    }
    // The entry has to stay inside the module, and it is read from a file the
    // module author controls — so it is validated on the same terms as a
    // requested asset path rather than trusted.
    if safe_relative_path(&manifest.entry).is_none() {
        return Err(format!("entry {:?} escapes the module", manifest.entry));
    }
    Ok(manifest)
}

/// Resolve a caller-supplied relative path to a normalized form, or `None` when
/// it is not confined to the module directory.
///
/// Purely lexical, and that is the point: it runs before any file system call,
/// so a traversal never reaches `canonicalize` (which would follow a symlink
/// out of the tree before anyone could check where it landed). The containment
/// check after canonicalization is the second of the two gates, not the only one.
fn safe_relative_path(path: &str) -> Option<PathBuf> {
    if path.is_empty() {
        return None;
    }
    let candidate = StdPath::new(path);
    let mut normalized = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(segment) => {
                // A NUL or a separator smuggled through percent-decoding would
                // land here as part of one segment.
                let text = segment.to_str()?;
                if text.contains('\0') {
                    return None;
                }
                normalized.push(text);
            }
            // `.` is harmless but pointless; everything else leaves the tree or
            // re-anchors the join.
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    if normalized.as_os_str().is_empty() {
        return None;
    }
    Some(normalized)
}

/// GET /api/modules — list runtime modules found in the data directory.
///
/// **Route:** `GET /api/modules`
///
/// Returns every directory under `<data_dir>/modules/`, valid or not. An empty
/// list means the directory holds nothing; it does not distinguish that from an
/// absent directory, because for a caller deciding what to render they are the
/// same state.
pub async fn list_modules(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;

    let root = modules_root();
    let Ok(dir) = std::fs::read_dir(&root) else {
        // No modules directory: nothing installed. Not an error.
        return ok_data(Vec::<ModuleEntry>::new());
    };

    let mut entries: Vec<ModuleEntry> = Vec::new();
    for item in dir.flatten() {
        if !item.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Some(id) = item.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if id.starts_with('.') {
            continue;
        }
        if !is_valid_module_id(&id) {
            entries.push(ModuleEntry {
                id,
                manifest: None,
                error: Some(
                    "directory name is not a valid module id (ASCII letters, digits, '-' and '_', at most 64)"
                        .to_string(),
                ),
            });
            continue;
        }
        match read_manifest(&item.path(), &id) {
            Ok(manifest) => entries.push(ModuleEntry {
                id,
                manifest: Some(manifest),
                error: None,
            }),
            Err(error) => entries.push(ModuleEntry {
                id,
                manifest: None,
                error: Some(error),
            }),
        }
    }

    entries.sort_by(|a, b| a.id.cmp(&b.id));
    ok_data(entries)
}

/// GET /api/modules/:id/assets/*path — serve one file from a module directory.
///
/// **Route:** `GET /api/modules/{id}/assets/{*path}`
///
/// Unlike the SPA fallback in [`super::assets`], a miss is a 404 rather than the
/// entry document: a module is not required to be a single-page app, and
/// answering every wrong path with HTML would turn a typo into a blank frame
/// instead of an error.
pub async fn serve_module_asset(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((id, path)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    check_auth(&state, &headers)?;

    if !is_valid_module_id(&id) {
        return Err(AppError::Validation("Invalid module id".to_string()));
    }
    let relative = safe_relative_path(&path)
        .ok_or_else(|| AppError::Validation("Invalid path".to_string()))?;

    let module_dir = modules_root().join(&id);
    let file_path = module_dir.join(&relative);

    // Second gate: resolve both sides and confirm the file really sits inside
    // the module. The lexical check above already rejected `..`; this catches a
    // symlink planted inside the module directory that points out of it.
    let canonical_dir = module_dir
        .canonicalize()
        .map_err(|_| AppError::NotFound("Module not found".to_string()))?;
    let canonical = file_path
        .canonicalize()
        .map_err(|_| AppError::NotFound("File not found".to_string()))?;
    if !canonical.starts_with(&canonical_dir) {
        return Err(AppError::Validation("Access denied".to_string()));
    }
    if !canonical.is_file() {
        return Err(AppError::NotFound("File not found".to_string()));
    }

    let data = tokio::fs::read(&canonical)
        .await
        .map_err(|_| AppError::NotFound("File not found".to_string()))?;

    let mime = mime_guess::from_path(&canonical).first_or_octet_stream();

    Ok((
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, mime.as_ref().to_string()),
            // A module is replaced by overwriting files in place, with no
            // content hash in the URL to invalidate a cached copy.
            (axum::http::header::CACHE_CONTROL, "no-cache".to_string()),
        ],
        data,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_ids_are_single_safe_segments() {
        assert!(is_valid_module_id("cil-console"));
        assert!(is_valid_module_id("a_1"));

        assert!(!is_valid_module_id(""));
        assert!(!is_valid_module_id("../etc"));
        assert!(!is_valid_module_id("a/b"));
        assert!(!is_valid_module_id("a\\b"));
        assert!(!is_valid_module_id("a b"));
        assert!(!is_valid_module_id("émoji"));
        assert!(!is_valid_module_id(&"x".repeat(65)));
    }

    #[test]
    fn relative_paths_that_leave_the_module_are_rejected() {
        assert_eq!(
            safe_relative_path("assets/app.js"),
            Some(PathBuf::from("assets/app.js"))
        );
        // `.` segments are dropped rather than rejected.
        assert_eq!(
            safe_relative_path("./assets/./app.js"),
            Some(PathBuf::from("assets/app.js"))
        );

        assert_eq!(safe_relative_path(""), None);
        assert_eq!(safe_relative_path(".."), None);
        assert_eq!(safe_relative_path("../secret"), None);
        assert_eq!(safe_relative_path("assets/../../secret"), None);
        assert_eq!(safe_relative_path("/etc/passwd"), None);
        assert_eq!(safe_relative_path("."), None);
    }

    #[test]
    fn a_manifest_may_not_claim_another_directorys_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let module = dir.path().join("mine");
        std::fs::create_dir(&module).expect("create module dir");
        std::fs::write(
            module.join(MANIFEST_NAME),
            r#"{"id": "theirs", "name": "Mine"}"#,
        )
        .expect("write manifest");

        let err = read_manifest(&module, "mine").expect_err("id mismatch must be rejected");
        assert!(err.contains("does not match directory"), "got: {err}");
    }

    #[test]
    fn the_smallest_manifest_is_a_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let module = dir.path().join("small");
        std::fs::create_dir(&module).expect("create module dir");
        std::fs::write(module.join(MANIFEST_NAME), r#"{"name": "Small"}"#).expect("write manifest");

        let manifest = read_manifest(&module, "small").expect("minimal manifest must be accepted");
        assert_eq!(manifest.name, "Small");
        assert_eq!(manifest.entry, "index.html");
        assert!(manifest.requires.is_empty());
    }

    #[test]
    fn an_entry_that_escapes_the_module_is_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let module = dir.path().join("escaper");
        std::fs::create_dir(&module).expect("create module dir");
        std::fs::write(
            module.join(MANIFEST_NAME),
            r#"{"name": "Escaper", "entry": "../../etc/passwd"}"#,
        )
        .expect("write manifest");

        let err = read_manifest(&module, "escaper").expect_err("escaping entry must be rejected");
        assert!(err.contains("escapes the module"), "got: {err}");
    }

    #[test]
    fn a_broken_manifest_reports_why() {
        let dir = tempfile::tempdir().expect("tempdir");
        let module = dir.path().join("broken");
        std::fs::create_dir(&module).expect("create module dir");
        std::fs::write(module.join(MANIFEST_NAME), "{not json").expect("write manifest");

        let err = read_manifest(&module, "broken").expect_err("broken manifest must be rejected");
        assert!(err.contains("invalid module.json"), "got: {err}");

        // A directory with no manifest at all is a different message, so an
        // operator can tell "I forgot the file" from "I wrote it wrong".
        let empty = dir.path().join("empty");
        std::fs::create_dir(&empty).expect("create module dir");
        let err = read_manifest(&empty, "empty").expect_err("missing manifest must be rejected");
        assert!(err.contains("cannot read module.json"), "got: {err}");
    }
}
