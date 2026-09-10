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

/// A module the kernel found, and where its files are.
///
/// The directory is carried alongside the listing row because the two routes
/// need the same answer and must not compute it twice: a listing that finds a
/// module in one place while the asset route looks in another is a 404 nobody
/// can explain. `root` is `None` for rows that name a problem rather than a
/// module — there is nothing to serve from.
struct Discovered {
    id: String,
    root: Option<PathBuf>,
    manifest: Option<ModuleManifest>,
    error: Option<String>,
}

impl Discovered {
    fn rejected(id: String, error: String) -> Self {
        Self {
            id,
            root: None,
            manifest: None,
            error: Some(error),
        }
    }

    fn into_entry(self) -> ModuleEntry {
        ModuleEntry {
            id: self.id,
            manifest: self.manifest,
            error: self.error,
        }
    }
}

/// Modules placed by hand under `<data_dir>/modules/`.
///
/// The path an operator uses while building one, and the only path there was
/// before connectors could ship panels. Kept for that: a module being developed
/// has no package to install from yet.
fn discover_placed_modules(root: &StdPath) -> Vec<Discovered> {
    let Ok(dir) = std::fs::read_dir(root) else {
        // No modules directory: nothing placed. Not an error.
        return Vec::new();
    };

    let mut found: Vec<Discovered> = Vec::new();
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
            found.push(Discovered::rejected(id, INVALID_ID_MESSAGE.to_string()));
            continue;
        }
        match read_manifest(&item.path(), &id) {
            Ok(manifest) => found.push(Discovered {
                id,
                root: Some(item.path()),
                manifest: Some(manifest),
                error: None,
            }),
            Err(error) => found.push(Discovered::rejected(id, error)),
        }
    }
    found
}

/// Panels declared by installed connectors.
///
/// The supported path: install brings the panel, uninstall takes it away, and
/// the version, the receipt and the seal are the connector's — none of which a
/// directory someone copied in has. The kernel reads the declaration from the
/// connector's own manifest (`managers::connector_manifest`), so a panel is
/// described in the same file that says what the connector is.
fn discover_connector_panels(servers_root: &StdPath) -> Vec<Discovered> {
    let Ok(dir) = std::fs::read_dir(servers_root) else {
        return Vec::new();
    };

    let mut found: Vec<Discovered> = Vec::new();
    for item in dir.flatten() {
        if !item.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Some(connector_id) = item.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if connector_id.starts_with('.') {
            continue;
        }
        let Some(declared) =
            crate::managers::connector_manifest::read_panels(servers_root, &connector_id)
        else {
            continue;
        };
        for panel in declared.panels {
            // The id the dashboard sees names both halves, because a panel id
            // is only unique inside its connector. It is built here and never
            // taken apart again: resolution matches whole ids against this
            // same enumeration, so a '-' inside either half cannot make one id
            // resolve as another.
            let id = format!("{connector_id}-{}", panel.id);
            if !is_valid_module_id(&panel.id) || !is_valid_module_id(&id) {
                found.push(Discovered::rejected(
                    id,
                    format!(
                        "connector {connector_id:?} declares panel id {:?}, which does not \
                         produce a valid module id ({INVALID_ID_MESSAGE})",
                        panel.id
                    ),
                ));
                continue;
            }
            if panel.name.trim().is_empty() {
                found.push(Discovered::rejected(
                    id,
                    format!("connector {connector_id:?} declares a panel with an empty name"),
                ));
                continue;
            }
            // Same terms as a placed module's manifest: the entry is written by
            // the connector author and has to stay inside the panel's root.
            if safe_relative_path(&panel.entry).is_none() {
                found.push(Discovered::rejected(
                    id,
                    format!("entry {:?} escapes the connector", panel.entry),
                ));
                continue;
            }
            found.push(Discovered {
                id,
                root: Some(declared.root.clone()),
                manifest: Some(ModuleManifest {
                    id: None,
                    name: panel.name,
                    description: panel.description,
                    version: panel.version,
                    entry: panel.entry,
                    icon: panel.icon,
                    requires: panel.requires,
                }),
                error: None,
            });
        }
    }
    found
}

const INVALID_ID_MESSAGE: &str =
    "not a valid module id (ASCII letters, digits, '-' and '_', at most 64)";

/// Every module the kernel can see, from both sources, sorted by id.
///
/// Two sources means two things can claim one id, and there is no ordering
/// between them that would be right: preferring the connector hides a module
/// an operator placed deliberately, preferring the placed one lets a stray
/// directory shadow an installed connector's panel. So neither wins — the id
/// is reported as ambiguous and serves nothing, which is the only outcome that
/// cannot be mistaken for the module someone meant.
fn discover_modules_in(modules_root: &StdPath, servers_root: Option<&StdPath>) -> Vec<Discovered> {
    let mut found = discover_placed_modules(modules_root);
    if let Some(servers_root) = servers_root {
        found.extend(discover_connector_panels(servers_root));
    }

    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for item in &found {
        *counts.entry(item.id.clone()).or_default() += 1;
    }

    let mut merged: Vec<Discovered> = Vec::new();
    let mut ambiguous_emitted: std::collections::HashSet<String> = std::collections::HashSet::new();
    for item in found {
        if counts.get(&item.id).copied().unwrap_or(0) > 1 {
            if ambiguous_emitted.insert(item.id.clone()) {
                merged.push(Discovered::rejected(
                    item.id,
                    "more than one source claims this id — nothing is served for it".to_string(),
                ));
            }
            continue;
        }
        merged.push(item);
    }

    merged.sort_by(|a, b| a.id.cmp(&b.id));
    merged
}

/// [`discover_modules_in`] against the roots this installation actually uses.
///
/// Kept to two lines with no decisions of its own: everything worth testing is
/// in the function it calls, which takes its roots as arguments so a test can
/// point it at a directory it made.
fn discover_modules() -> Vec<Discovered> {
    let servers_root = crate::managers::mcp_venv::resolve_servers_dir_from_config();
    discover_modules_in(&modules_root(), servers_root.as_deref())
}

/// GET /api/modules — list runtime modules, from both sources.
///
/// **Route:** `GET /api/modules`
///
/// Returns every module found under `<data_dir>/modules/` and every panel an
/// installed connector declares, valid or not. An empty list means neither
/// source holds anything; it does not distinguish that from an absent
/// directory, because for a caller deciding what to render they are the same
/// state.
pub async fn list_modules(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    check_auth(&state, &headers)?;
    ok_data(
        discover_modules()
            .into_iter()
            .map(Discovered::into_entry)
            .collect::<Vec<_>>(),
    )
}

/// Second gate: resolve both sides and confirm the file really sits inside the
/// module. The lexical check in [`safe_relative_path`] already rejected `..`;
/// this catches a symlink planted inside the module directory that points out
/// of it.
///
/// Shared by both sources on purpose. A connector's panel root is a directory
/// the kernel did not write either, so it needs the same gate — and a second
/// copy of this check is a second thing that can be weakened alone.
fn resolve_asset(module_dir: &StdPath, relative: &StdPath) -> Result<PathBuf, AppError> {
    let file_path = module_dir.join(relative);
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
    Ok(canonical)
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

    // Resolved through the same enumeration the listing uses, so a module is
    // served from where it was listed from — and an id that resolves to
    // nothing (unknown, rejected, or claimed by two sources) serves nothing.
    let module_dir = discover_modules()
        .into_iter()
        .find(|m| m.id == id)
        .and_then(|m| m.root)
        .ok_or_else(|| AppError::NotFound("Module not found".to_string()))?;
    let canonical = resolve_asset(&module_dir, &relative)?;

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

#[cfg(test)]
mod connector_panel_tests {
    use super::*;

    /// A connector as the installer leaves it: a directory under the servers
    /// root with its manifest inside. `nested` is the layout the current
    /// installer produces; the flat one is what earlier installs left behind,
    /// and both are on disk at once.
    fn install_connector(
        servers_root: &StdPath,
        id: &str,
        nested: bool,
        manifest: &str,
    ) -> PathBuf {
        let dir = if nested {
            servers_root.join(id).join("servers").join(id)
        } else {
            servers_root.join(id)
        };
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("cloto-connector.json"), manifest).unwrap();
        dir
    }

    const ONE_PANEL: &str = r#"{
        "spec_version": 1,
        "ui": { "panels": [ { "id": "console", "name": "Operating Console",
                              "entry": "ui/index.html",
                              "requires": ["GET /api/published/cil"] } ] }
    }"#;

    fn ids(found: &[Discovered]) -> Vec<&str> {
        found.iter().map(|m| m.id.as_str()).collect()
    }

    #[test]
    fn a_connector_panel_is_listed_under_an_id_naming_both_halves() {
        let modules = tempfile::tempdir().unwrap();
        let servers = tempfile::tempdir().unwrap();
        install_connector(servers.path(), "cil", false, ONE_PANEL);

        let found = discover_modules_in(modules.path(), Some(servers.path()));

        assert_eq!(ids(&found), vec!["cil-console"]);
        let manifest = found[0].manifest.as_ref().expect("a usable panel");
        assert_eq!(manifest.name, "Operating Console");
        assert_eq!(manifest.entry, "ui/index.html");
        assert_eq!(
            manifest.requires,
            vec!["GET /api/published/cil".to_string()]
        );
    }

    #[test]
    fn the_layout_earlier_installs_left_behind_is_read_too() {
        let modules = tempfile::tempdir().unwrap();
        let servers = tempfile::tempdir().unwrap();
        let dir = install_connector(servers.path(), "cil", true, ONE_PANEL);

        let found = discover_modules_in(modules.path(), Some(servers.path()));

        assert_eq!(ids(&found), vec!["cil-console"]);
        assert_eq!(
            found[0].root.as_deref(),
            Some(dir.as_path()),
            "assets resolve against the directory holding the manifest"
        );
    }

    #[test]
    fn a_connector_that_declares_no_panels_contributes_nothing() {
        let modules = tempfile::tempdir().unwrap();
        let servers = tempfile::tempdir().unwrap();
        install_connector(servers.path(), "plain", false, r#"{"spec_version": 1}"#);

        assert!(discover_modules_in(modules.path(), Some(servers.path())).is_empty());
    }

    #[test]
    fn a_connector_this_kernel_refuses_to_run_gets_no_surface() {
        let modules = tempfile::tempdir().unwrap();
        let servers = tempfile::tempdir().unwrap();
        install_connector(
            servers.path(),
            "future",
            false,
            r#"{"spec_version": 1, "connector_type": "something_newer",
                "ui": { "panels": [ { "id": "p", "name": "P" } ] } }"#,
        );

        assert!(
            discover_modules_in(modules.path(), Some(servers.path())).is_empty(),
            "a connector the kernel will not launch must not get a panel either"
        );
    }

    #[test]
    fn a_panel_whose_entry_escapes_its_connector_is_reported_not_served() {
        let modules = tempfile::tempdir().unwrap();
        let servers = tempfile::tempdir().unwrap();
        install_connector(
            servers.path(),
            "cil",
            false,
            r#"{"ui": { "panels": [ { "id": "p", "name": "P",
                                      "entry": "../../../etc/passwd" } ] } }"#,
        );

        let found = discover_modules_in(modules.path(), Some(servers.path()));
        assert_eq!(ids(&found), vec!["cil-p"]);
        assert!(found[0].error.is_some(), "the reason is reported");
        assert!(found[0].root.is_none(), "and nothing is served for it");
    }

    #[test]
    fn when_two_sources_claim_one_id_neither_wins() {
        let modules = tempfile::tempdir().unwrap();
        let servers = tempfile::tempdir().unwrap();

        // A placed module named exactly what the connector's panel resolves to.
        let placed = modules.path().join("cil-console");
        std::fs::create_dir_all(&placed).unwrap();
        std::fs::write(placed.join("module.json"), r#"{"name": "Placed"}"#).unwrap();
        install_connector(servers.path(), "cil", false, ONE_PANEL);

        let found = discover_modules_in(modules.path(), Some(servers.path()));

        assert_eq!(ids(&found), vec!["cil-console"], "one row, not two");
        assert!(found[0].root.is_none(), "an ambiguous id serves nothing");
        assert!(found[0].manifest.is_none());
        assert!(found[0].error.is_some());
    }

    #[test]
    fn a_placed_module_and_a_panel_with_distinct_ids_both_appear() {
        let modules = tempfile::tempdir().unwrap();
        let servers = tempfile::tempdir().unwrap();
        let placed = modules.path().join("scratch");
        std::fs::create_dir_all(&placed).unwrap();
        std::fs::write(placed.join("module.json"), r#"{"name": "Scratch"}"#).unwrap();
        install_connector(servers.path(), "cil", false, ONE_PANEL);

        let found = discover_modules_in(modules.path(), Some(servers.path()));

        assert_eq!(ids(&found), vec!["cil-console", "scratch"]);
    }

    #[test]
    fn with_no_servers_root_only_placed_modules_are_listed() {
        let modules = tempfile::tempdir().unwrap();
        let placed = modules.path().join("scratch");
        std::fs::create_dir_all(&placed).unwrap();
        std::fs::write(placed.join("module.json"), r#"{"name": "Scratch"}"#).unwrap();

        assert_eq!(
            ids(&discover_modules_in(modules.path(), None)),
            vec!["scratch"]
        );
    }

    /// Both routes have to resolve a module through the same enumeration, and
    /// nothing above can see whether they still do: every test in this file
    /// drives `discover_modules_in` directly, so a handler rewritten to walk
    /// `<data_dir>/modules/` by itself again would leave them all green while
    /// connector panels quietly stopped being served. Reading the source is the
    /// only place that question can be asked.
    #[test]
    fn both_routes_resolve_a_module_through_the_shared_discovery() {
        // Only the half above the tests: this test names the function it is
        // counting, so measuring the whole file would count itself.
        let source = include_str!("modules.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("split always yields the head");
        let definitions = production.matches("fn discover_modules()").count();
        let mentions = production.matches("discover_modules()").count();

        assert_eq!(definitions, 1, "expected exactly one definition");
        assert_eq!(
            mentions - definitions,
            2,
            "expected the listing and the asset route to be its only two callers"
        );
    }

    // ── the second gate, on a connector's root ──

    #[test]
    fn a_file_inside_the_panel_resolves() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("ui")).unwrap();
        std::fs::write(dir.path().join("ui").join("index.html"), "<h1>hi</h1>").unwrap();

        let resolved = resolve_asset(dir.path(), StdPath::new("ui/index.html"));
        let Ok(resolved) = resolved else {
            panic!("a real file inside the panel should resolve");
        };
        assert!(resolved.is_file());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_planted_in_the_panel_cannot_reach_outside_it() {
        // The lexical gate cannot see this one: the requested path has no `..`
        // in it. Only resolving both sides answers where the file really is.
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "not yours").unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), dir.path().join("escape"))
            .unwrap();

        let refused = resolve_asset(dir.path(), StdPath::new("escape"));

        assert!(
            matches!(refused, Err(AppError::Validation(_))),
            "the containment check must refuse a symlink that leaves the panel"
        );
    }
}
