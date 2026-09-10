//! Characterization tests for the marketplace `raw_url` install path.
//!
//! These pin what the installer does at the boundaries: what is fetched
//! and how it is verified, where the tree lands on disk, which commands
//! the Python environment step runs, what each failure leaves behind, and
//! — above all — what is written to the database and when. A change in
//! behaviour belongs in its own commit with the expectation updated first.
//!
//! The path runs through the install engine (`tools/cloto-installer`),
//! which these tests build from source once and hand to the kernel via
//! `CLOTO_INSTALLER`; the kernel's side of the contract — inputs, event
//! forwarding, the verdict, registration — is what is exercised here.
//!
//! The download stage refuses loopback addresses (its SSRF guard), so the
//! whole path cannot be driven end to end against a local server. The
//! tests drive the two stages `install_from_raw_url` is made of —
//! `fetch_raw_url_archive` pinned to the local server's address, then
//! `materialize_with_installer` — exactly as the production wrapper chains
//! them, and pin the guard itself through `run_install`. The Python
//! environment step is observed through a stand-in `uv` that records its
//! arguments, which is why this file is Unix-only.
#![cfg(unix)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use axum::extract::{ConnectInfo, Query, State};
use axum::http::HeaderMap;
use cloto_core::handlers::marketplace::{
    catalog_handler, fetch_raw_url_archive, install_handler, materialize_with_installer,
    run_install, CatalogQuery, InstallOutcome, InstallRequest, RegistryEntry,
};
use cloto_core::handlers::setup::SetupProgressEvent;
use cloto_core::managers::installer::{self, InstallerState};
use cloto_core::test_utils::create_test_app_state_in;
use cloto_core::AppState;
use mgp_sdk::adapters::{RawUrlSpec, SourceSpec};
use mgp_sdk::shape::InstallShape;
use tokio::sync::Mutex;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// `CLOTO_CATALOG_URL` / `CLOTO_SEAL_JWKS_URL` are process-global and every
/// test points them at its own mock server, so tests run one at a time.
static ENV_LOCK: Mutex<()> = Mutex::const_new(());

const API_KEY: &str = "test-key";
const KID: &str = "test-hub-key";

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "clotocore-install-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(data))
}

/// A gzipped tarball with the given entries, `git archive`-style: a global
/// pax header naming the commit first (as GitHub and the hub serve them —
/// an entry the extractor must not mistake for a top-level file), then one
/// shared top-level directory, which the installer strips.
fn tarball(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut tar_buf = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_buf);
        let record = b"52 comment=d2368156b0f34f1c7930400cb4d35ed77c2eafb3\n";
        let mut global = tar::Header::new_ustar();
        global.set_entry_type(tar::EntryType::XGlobalHeader);
        global.set_path("pax_global_header").unwrap();
        global.set_mode(0o644);
        global.set_size(record.len() as u64);
        global.set_cksum();
        builder.append(&global, &record[..]).unwrap();
        for (name, data) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            builder.append_data(&mut header, name, *data).unwrap();
        }
        builder.finish().unwrap();
    }
    use std::io::Write;
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&tar_buf).unwrap();
    gz.finish().unwrap()
}

/// Place a stand-in `uv` under `{data_dir}/bin/`. It appends every
/// invocation's arguments to `uv-calls.log`, creates a plausible venv on
/// `uv venv`, and (optionally) fails every `uv pip` call.
fn install_fake_uv(data_dir: &Path, fail_pip: bool) -> PathBuf {
    let bin = data_dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let log = data_dir.join("uv-calls.log");
    let fail = if fail_pip {
        "if [ \"$1\" = \"pip\" ]; then echo 'simulated dependency failure' >&2; exit 1; fi\n"
    } else {
        ""
    };
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"{log}\"\n\
         if [ \"$1\" = \"venv\" ]; then mkdir -p \"$4/bin\" && printf 'version_info = 3.13.3\\n' > \"$4/pyvenv.cfg\"; fi\n\
         {fail}exit 0\n",
        log = log.display(),
    );
    let uv = bin.join("uv");
    std::fs::write(&uv, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&uv, std::fs::Permissions::from_mode(0o755)).unwrap();
    log
}

fn uv_calls(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log)
        .map(|s| s.lines().map(str::to_owned).collect())
        .unwrap_or_default()
}

/// The install engine, built from `tools/cloto-installer` once per test
/// process and stamped with this crate's version — the version the kernel
/// requires of it. Needs a Go toolchain on `PATH`; the build failing is a
/// test failure, not a skip, so a missing toolchain cannot quietly turn
/// this file into a no-op.
fn install_engine() -> &'static Path {
    static ENGINE: OnceLock<PathBuf> = OnceLock::new();
    ENGINE.get_or_init(|| {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/cloto-installer");
        let out_dir =
            std::env::temp_dir().join(format!("clotocore-install-engine-{}", std::process::id()));
        std::fs::create_dir_all(&out_dir).unwrap();
        let out = out_dir.join("cloto-installer");
        let status = std::process::Command::new("go")
            .args(["build", "-trimpath", "-ldflags"])
            .arg(format!("-X main.version={}", env!("CARGO_PKG_VERSION")))
            .arg("-o")
            .arg(&out)
            .arg(".")
            .current_dir(&source)
            .status()
            .expect("`go` is required to build the install engine for these tests");
        assert!(status.success(), "go build of the install engine failed");
        out
    })
}

struct Hub {
    signing: mgp_seal::ed25519::PrivateKey,
    jwks: serde_json::Value,
}

impl Hub {
    fn new() -> Self {
        let (sk, pk) = mgp_seal::ed25519::generate_keypair(&mut rand::rngs::OsRng);
        let kid = mgp_seal::ed25519::KeyId::new(KID).unwrap();
        let jwks = serde_json::json!({ "keys": [mgp_seal::ed25519::public_key_to_jwk(&pk, &kid)] });
        Self { signing: sk, jwks }
    }

    /// A catalog entry the hub would publish for `archive`: entry-point hash
    /// recorded, archive digest and length bound, Ed25519-signed.
    #[allow(clippy::too_many_arguments)]
    fn entry(
        &self,
        id: &str,
        directory: &str,
        version: &str,
        server_py: &[u8],
        archive: &[u8],
        url: &str,
        subdir: Option<&str>,
        dependencies: &[&str],
    ) -> RegistryEntry {
        let entry_point_sha256 = sha256_hex(server_py);
        let archive_sha256 = sha256_hex(archive);
        let canonical = mgp_seal::canonical_message_v2(
            id,
            version,
            &entry_point_sha256,
            &archive_sha256,
            archive.len() as u64,
        );
        let kid = mgp_seal::ed25519::KeyId::new(KID).unwrap();
        let sig = mgp_seal::ed25519::sign(&self.signing, &kid, &canonical);
        RegistryEntry {
            id: id.into(),
            name: "Demo".into(),
            description: "demo connector".into(),
            category: "tool".into(),
            version: version.into(),
            directory: directory.into(),
            dependencies: dependencies.iter().map(|d| (*d).to_owned()).collect(),
            env_vars: vec![],
            optional_env_vars: vec![],
            tags: vec![],
            trust_level: "standard".into(),
            auto_restart: false,
            icon: None,
            runtime: "python".into(),
            bin_name: None,
            changelog: None,
            seal: None,
            entry_point_sha256: Some(entry_point_sha256),
            signature_payload: Some(serde_json::json!({
                "ed25519": { "sig": sig.to_base64(), "key_id": KID },
                "archive": { "sha256": archive_sha256, "length": archive.len() },
            })),
            install: Some(InstallShape {
                source: SourceSpec::RawUrl(RawUrlSpec {
                    url: url.into(),
                    sha256: Some(archive_sha256),
                    subdir: subdir.map(str::to_owned),
                }),
                package_manager: Some("uv".into()),
            }),
            provider: None,
        }
    }
}

struct Harness {
    state: Arc<AppState>,
    data_dir: PathBuf,
    mock: MockServer,
    hub: Hub,
    uv_log: PathBuf,
    events: tokio::sync::broadcast::Receiver<SetupProgressEvent>,
}

impl Harness {
    async fn new(tag: &str, fail_pip: bool) -> Self {
        let data_dir = temp_dir(tag);
        let uv_log = install_fake_uv(&data_dir, fail_pip);
        let state = create_test_app_state_in(data_dir.clone(), Some(API_KEY.into())).await;
        let mock = MockServer::start().await;
        let hub = Hub::new();
        Mock::given(method("GET"))
            .and(path("/api/seal/keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(hub.jwks.clone()))
            .mount(&mock)
            .await;
        std::env::set_var("CLOTO_CATALOG_URL", format!("{}/api/catalog", mock.uri()));
        std::env::set_var(
            "CLOTO_SEAL_JWKS_URL",
            format!("{}/api/seal/keys", mock.uri()),
        );
        std::env::set_var(installer::ENV_OVERRIDE, install_engine());
        let events = state.setup_progress_tx.subscribe();
        Self {
            state,
            data_dir,
            mock,
            hub,
            uv_log,
            events,
        }
    }

    fn archive_url(&self, name: &str) -> String {
        format!("{}/dl/{name}", self.mock.uri())
    }

    async fn serve_archive(&self, name: &str, body: Vec<u8>) {
        Mock::given(method("GET"))
            .and(path(format!("/dl/{name}")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
            .mount(&self.mock)
            .await;
    }

    async fn serve_catalog(&self, entries: &[&RegistryEntry]) {
        // The catalog handler reads the cached document first; a fresh mock
        // route plus `force_refresh` makes it re-fetch.
        let body = serde_json::json!({
            "schema_version": 1,
            "updated_at": "2026-01-01T00:00:00Z",
            "servers": entries,
            "collections": [],
        });
        Mock::given(method("GET"))
            .and(path("/api/catalog"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&self.mock)
            .await;
    }

    /// The two stages of `install_from_raw_url`, chained as it chains them,
    /// with the download pinned to the local server's address (the guard
    /// that would refuse it is exercised separately through `run_install`).
    async fn download_and_materialize(
        &self,
        entry: &RegistryEntry,
    ) -> anyhow::Result<InstallOutcome> {
        self.download_and_materialize_starting(entry, false).await
    }

    /// `download_and_materialize` with control over `auto_start`, for the
    /// tests that need the installed server registered and running.
    async fn download_and_materialize_starting(
        &self,
        entry: &RegistryEntry,
        auto_start: bool,
    ) -> anyhow::Result<InstallOutcome> {
        let Some(InstallShape {
            source: SourceSpec::RawUrl(spec),
            ..
        }) = entry.install.as_ref()
        else {
            panic!("entry is not a raw_url source");
        };
        let tmp_dir = self.data_dir.join("tmp");
        tokio::fs::create_dir_all(&tmp_dir).await?;
        let archive_path = tmp_dir.join(format!("{}-raw-url.tar.gz", entry.id));
        let engine = install_engine();
        if !fetch_raw_url_archive(
            &self.state.setup_progress_tx,
            engine,
            entry,
            &[*self.mock.address()],
            &archive_path,
        )
        .await?
        {
            return Ok(InstallOutcome::NotInstalled);
        }
        materialize_with_installer(
            &self.state,
            engine,
            entry,
            spec.subdir.as_deref(),
            &archive_path,
            &tmp_dir,
            HashMap::new(),
            auto_start,
        )
        .await
    }

    /// Drain the progress events emitted so far, as compact labels.
    /// `StepProgress` is dropped: its count depends on chunking.
    fn steps(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(ev) = self.events.try_recv() {
            let label = match ev {
                SetupProgressEvent::StepStart { step, .. } => format!("start:{step}"),
                SetupProgressEvent::StepComplete { step } => format!("complete:{step}"),
                SetupProgressEvent::StepError {
                    step,
                    error,
                    recoverable,
                } => format!(
                    "error:{step}:{}:{error}",
                    if recoverable { "recoverable" } else { "fatal" }
                ),
                SetupProgressEvent::ServerInstall {
                    server_name,
                    status,
                } => format!("install:{server_name}:{status}"),
                SetupProgressEvent::StepProgress { .. } => continue,
                SetupProgressEvent::Complete => "complete".into(),
            };
            out.push(label);
        }
        out
    }

    fn servers_dir(&self) -> PathBuf {
        self.data_dir.join("mcp-servers")
    }

    /// The venv the installer targets: an existing venv found by the global
    /// resolver, else the shared one under this data dir. The resolver reads
    /// process-global state (the running binary's location), which the
    /// tests record rather than hide — it is an input the boundary carries.
    fn venv_dir(&self) -> PathBuf {
        let venv = cloto_core::managers::mcp_venv::resolve_venv_dir()
            .unwrap_or_else(|| self.servers_dir().join(".venv"));
        // The resolver can hand back an unnormalized path (`repo/../x`) on a
        // development machine; the engine joins it lexically, so `uv` sees
        // the cleaned form.
        let mut clean = PathBuf::new();
        for component in venv.components() {
            match component {
                std::path::Component::ParentDir => {
                    clean.pop();
                }
                std::path::Component::CurDir => {}
                other => clean.push(other),
            }
        }
        clean
    }

    async fn db_row(&self, id: &str) -> Option<DbRow> {
        sqlx::query_as::<_, DbRow>(
            "SELECT name, installed_version, marketplace_id, trust_level, seal, is_active \
             FROM mcp_servers WHERE name = ?",
        )
        .bind(id)
        .fetch_optional(&self.state.pool)
        .await
        .unwrap()
    }

    /// The catalog view's three-state answer for `id`.
    async fn catalog_state(&self, id: &str) -> serde_json::Value {
        let mut headers = HeaderMap::new();
        headers.insert("X-API-Key", API_KEY.parse().unwrap());
        let axum::Json(body) = catalog_handler(
            State(self.state.clone()),
            headers,
            Query(CatalogQuery {
                force_refresh: true,
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("catalog handler returned an error"));
        let servers = body
            .pointer("/data/servers")
            .or_else(|| body.get("servers"))
            .and_then(serde_json::Value::as_array)
            .cloned()
            .expect("servers array in catalog response");
        let row = servers
            .into_iter()
            .find(|s| s["id"] == id)
            .expect("entry in catalog view");
        serde_json::json!({
            "installed": row["installed"],
            "installed_version": row["installed_version"],
            "update_available": row["update_available"],
            "running": row["running"],
        })
    }
}

#[derive(Debug, sqlx::FromRow)]
struct DbRow {
    name: String,
    installed_version: Option<String>,
    marketplace_id: Option<String>,
    trust_level: Option<String>,
    seal: Option<String>,
    is_active: bool,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.data_dir);
    }
}

const SERVER_PY: &[u8] = b"import sys\nsys.exit(0)\n";
const PYPROJECT: &[u8] = b"[project]\nname = \"demo\"\nversion = \"0.1.0\"\n";

fn standalone_archive(server_py: &[u8]) -> Vec<u8> {
    tarball(&[
        ("demo-1.0.0/server.py", server_py),
        ("demo-1.0.0/pyproject.toml", PYPROJECT),
        ("demo-1.0.0/pkg/__init__.py", b""),
    ])
}

// ── happy paths ──────────────────────────────────────────────────────

const PANEL_HTML: &[u8] = b"<!doctype html><title>panel</title>\n";
const PANEL_MANIFEST: &[u8] = br#"{"spec_version":1,"connector_type":"ui_module","id":"demo","name":"Demo","ui":{"panels":[{"id":"console","name":"Console","entry":"index.html"}]}}"#;

fn panel_archive() -> Vec<u8> {
    tarball(&[
        ("demo-1.0.0/cloto-connector.json", PANEL_MANIFEST),
        ("demo-1.0.0/index.html", PANEL_HTML),
    ])
}

/// A connector that ships a face and no server, installed the way every
/// published connector is: the hub rewrites a git source into a served
/// archive, so this path is the only one a catalog install takes.
///
/// It is also where the type is decided. The catalog entry this test builds
/// says `runtime: "python"`, exactly as it would for a server — the decision
/// comes from the manifest in the extracted tree, not from anything the
/// catalog claims, which is why the catalog being lossy about
/// `connector_type` costs nothing here.
#[tokio::test]
async fn a_connector_that_ships_no_server_installs_its_files_and_registers_nothing() {
    let _guard = ENV_LOCK.lock().await;
    let h = Harness::new("panel", false).await;
    let archive = panel_archive();
    let url = h.archive_url("panel.tar.gz");
    let entry = h
        .hub
        .entry("demo", "", "1.0.0", PANEL_HTML, &archive, &url, None, &[]);
    h.serve_archive("panel.tar.gz", archive).await;
    h.serve_catalog(&[&entry]).await;

    let outcome = h.download_and_materialize(&entry).await.unwrap();
    assert!(matches!(outcome, InstallOutcome::Installed), "{outcome:?}");

    // No dependency work was attempted. Not "it succeeded" — it was never
    // reached, which is what stops a missing `pyproject.toml` from being a
    // failure for a connector that was never going to have one.
    assert!(
        uv_calls(&h.uv_log).is_empty(),
        "uv ran for a connector with nothing to build: {:?}",
        uv_calls(&h.uv_log)
    );

    // The files are the whole connector, and they are in place.
    let install_dir = h.servers_dir().join("demo");
    assert_eq!(
        std::fs::read(install_dir.join("index.html")).unwrap(),
        PANEL_HTML
    );
    assert!(install_dir.join("cloto-connector.json").is_file());

    // Nothing is registered: there is no process to start, so a row would
    // describe a server that does not exist and the spawn path would treat
    // a connector with no command as one whose command failed.
    assert!(
        h.db_row("demo").await.is_none(),
        "a connector with no server was registered as one"
    );
}

#[tokio::test]
async fn standalone_archive_is_verified_extracted_built_and_registered() {
    let _guard = ENV_LOCK.lock().await;
    let mut h = Harness::new("standalone", false).await;
    let archive = standalone_archive(SERVER_PY);
    let url = h.archive_url("demo.tar.gz");
    let entry = h
        .hub
        .entry("demo", "", "1.0.0", SERVER_PY, &archive, &url, None, &[]);
    h.serve_archive("demo.tar.gz", archive).await;
    h.serve_catalog(&[&entry]).await;

    let venv_before = h.venv_dir().join("pyvenv.cfg").exists();
    h.download_and_materialize(&entry).await.unwrap();

    // Events, in order (progress ticks aside).
    // `start:download` is emitted by the wrapper before this stage runs,
    // so it is not observed here (see `run_install_refuses_loopback_...`).
    assert_eq!(
        h.steps(),
        [
            "complete:download",
            "start:extract",
            "complete:extract",
            "start:install_deps",
            "install:Demo:installing",
            "install:Demo:installed",
            "complete:install_deps",
            "start:finalize",
            "complete:finalize",
        ]
    );

    // On disk: the tree under mcp-servers/<id> with the archive's top-level
    // directory stripped; no staging directory and no archive left in tmp.
    let install_dir = h.servers_dir().join("demo");
    assert_eq!(
        std::fs::read(install_dir.join("server.py")).unwrap(),
        SERVER_PY
    );
    assert!(install_dir.join("pkg/__init__.py").is_file());
    assert!(!h.data_dir.join("tmp/demo-staging").exists());
    assert!(!h.data_dir.join("tmp/demo-raw-url.tar.gz").exists());
    assert_eq!(
        std::fs::read_dir(h.data_dir.join("tmp")).unwrap().count(),
        0
    );

    // The Python environment step: create the shared venv only when it
    // does not exist yet, then install the server tree into it. The
    // install runs against the staged tree, before it is swapped into
    // place, so the path `uv` sees is the staging path; the command
    // registered below names the final location.
    let venv = h.venv_dir();
    let staging_dir = h.data_dir.join("tmp/demo-staging");
    let mut expected_uv = Vec::new();
    if !venv_before {
        expected_uv.push(format!("venv --python 3.13 {}", venv.display()));
    }
    expected_uv.push(format!(
        "pip install --no-progress --python {} {}",
        venv.join("bin/python").display(),
        staging_dir.display()
    ));
    assert_eq!(uv_calls(&h.uv_log), expected_uv);

    // The database row: written by registration, carrying the marketplace
    // identity, the catalog version, the declared trust tier and a local
    // tree seal over the installed files.
    let row = h.db_row("demo").await.expect("registered row");
    assert_eq!(row.name, "demo");
    assert_eq!(row.installed_version.as_deref(), Some("1.0.0"));
    assert_eq!(row.marketplace_id.as_deref(), Some("demo"));
    assert_eq!(row.trust_level.as_deref(), Some("standard"));
    assert!(row.is_active);
    let seal = row.seal.expect("local seal minted");
    assert!(seal.starts_with("tree-sha256:"), "{seal}");
    let seal_key = std::fs::read(h.data_dir.join("seal.key")).unwrap();
    assert!(
        cloto_core::managers::tree_seal::verify_tree_seal(&install_dir, &seal, &seal_key).unwrap()
    );

    // The catalog view: installed at this version, nothing newer, not running
    // (the stand-in venv has no interpreter, so the connect attempt failed —
    // which registration tolerates).
    assert_eq!(
        h.catalog_state("demo").await,
        serde_json::json!({
            "installed": true,
            "installed_version": "1.0.0",
            "update_available": false,
            "running": false,
        })
    );

    // A newer catalog version flips update_available without touching the row.
    let newer = h
        .hub
        .entry("demo", "", "1.1.0", SERVER_PY, &[], &url, None, &[]);
    h.mock.reset().await; // earlier mounts on the same path take precedence
    h.serve_catalog(&[&newer]).await;
    assert_eq!(h.catalog_state("demo").await["update_available"], true);
}

#[tokio::test]
async fn monorepo_subdir_archive_keeps_repo_relative_layout_and_installs_common_first() {
    let _guard = ENV_LOCK.lock().await;
    let mut h = Harness::new("subdir", false).await;
    let archive = tarball(&[
        ("repo-v0/README.md", b"readme"),
        ("repo-v0/servers/demo/server.py", SERVER_PY),
        ("repo-v0/servers/demo/pyproject.toml", PYPROJECT),
        ("repo-v0/servers/common/pyproject.toml", PYPROJECT),
        ("repo-v0/servers/common/common/__init__.py", b""),
        ("repo-v0/servers/other/server.py", b"print('other')"),
    ]);
    let url = h.archive_url("mono.tar.gz");
    let entry = h.hub.entry(
        "demo",
        "servers/demo",
        "1.0.0",
        SERVER_PY,
        &archive,
        &url,
        Some("servers/demo"),
        &["common"],
    );
    h.serve_archive("mono.tar.gz", archive).await;

    h.download_and_materialize(&entry).await.unwrap();
    assert!(
        h.steps().contains(&"complete:finalize".to_string()),
        "install did not reach registration"
    );

    // A multi-segment catalog `directory` collapses to its last component;
    // inside it the connector keeps its repo-relative path, with the
    // declared `common` sibling alongside and nothing else from the repo.
    let install_dir = h.servers_dir().join("demo");
    let server_path = install_dir.join("servers/demo");
    assert!(server_path.join("server.py").is_file());
    assert!(install_dir
        .join("servers/common/common/__init__.py")
        .is_file());
    assert!(!install_dir.join("README.md").exists());
    assert!(!install_dir.join("servers/other").exists());
    assert!(!h.servers_dir().join("servers").exists());

    // `common` is installed into the venv before the connector itself,
    // both from the staged tree (see the standalone test).
    let venv = h.venv_dir();
    let python = venv.join("bin/python").display().to_string();
    let staging_dir = h.data_dir.join("tmp/demo-staging");
    let calls = uv_calls(&h.uv_log);
    let pip_calls: Vec<&String> = calls.iter().filter(|c| c.starts_with("pip ")).collect();
    assert_eq!(
        pip_calls,
        [
            &format!(
                "pip install --no-progress --python {python} {}",
                staging_dir.join("servers/common").display()
            ),
            &format!(
                "pip install --no-progress --python {python} {}",
                staging_dir.join("servers/demo").display()
            ),
        ]
    );

    let row = h.db_row("demo").await.expect("registered row");
    assert_eq!(row.installed_version.as_deref(), Some("1.0.0"));
    assert!(row.seal.is_some_and(|s| s.starts_with("tree-sha256:")));
}

#[tokio::test]
async fn reinstalling_a_newer_version_replaces_the_tree_and_updates_the_row() {
    let _guard = ENV_LOCK.lock().await;
    let mut h = Harness::new("update", false).await;
    let url = h.archive_url("demo.tar.gz");

    let venv_before = h.venv_dir().join("pyvenv.cfg").exists();
    let v1 = standalone_archive(SERVER_PY);
    let e1 = h
        .hub
        .entry("demo", "", "1.0.0", SERVER_PY, &v1, &url, None, &[]);
    h.serve_archive("demo.tar.gz", v1).await;
    h.download_and_materialize(&e1).await.unwrap();
    std::fs::write(h.servers_dir().join("demo/leftover.txt"), b"stale").unwrap();
    h.steps();

    let new_py: &[u8] = b"import sys\nsys.exit(0)  # v2\n";
    let v2 = standalone_archive(new_py);
    let e2 = h
        .hub
        .entry("demo", "", "2.0.0", new_py, &v2, &url, None, &[]);
    h.mock.reset().await;
    h.serve_archive("demo.tar.gz", v2).await;
    Mock::given(method("GET"))
        .and(path("/api/seal/keys"))
        .respond_with(ResponseTemplate::new(200).set_body_json(h.hub.jwks.clone()))
        .mount(&h.mock)
        .await;
    h.download_and_materialize(&e2).await.unwrap();
    assert!(h.steps().contains(&"complete:finalize".to_string()));

    // The staged tree replaces the old one wholesale: nothing from the
    // previous install survives, not even files the archive never had.
    let install_dir = h.servers_dir().join("demo");
    assert_eq!(
        std::fs::read(install_dir.join("server.py")).unwrap(),
        new_py
    );
    assert!(!install_dir.join("leftover.txt").exists());

    // The venv is created once; each install runs its own pip step.
    let calls = uv_calls(&h.uv_log);
    assert_eq!(
        calls.iter().filter(|c| c.starts_with("venv ")).count(),
        usize::from(!venv_before)
    );
    assert_eq!(calls.iter().filter(|c| c.starts_with("pip ")).count(), 2);

    let row = h.db_row("demo").await.expect("row");
    assert_eq!(row.installed_version.as_deref(), Some("2.0.0"));
    assert!(row.is_active);
}

// ── failure paths: what is emitted, what is left behind, what is NOT written ──

#[tokio::test]
async fn http_error_is_recoverable_and_leaves_nothing_behind() {
    let _guard = ENV_LOCK.lock().await;
    let mut h = Harness::new("http500", false).await;
    let archive = standalone_archive(SERVER_PY);
    let url = h.archive_url("demo.tar.gz");
    let entry = h
        .hub
        .entry("demo", "", "1.0.0", SERVER_PY, &archive, &url, None, &[]);
    Mock::given(method("GET"))
        .and(path("/dl/demo.tar.gz"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&h.mock)
        .await;

    h.download_and_materialize(&entry).await.unwrap();

    let steps = h.steps();
    assert_eq!(steps.len(), 1, "{steps:?}");
    assert!(
        steps[0].starts_with("error:download:recoverable:HTTP 503"),
        "{}",
        steps[0]
    );
    assert!(!h.servers_dir().join("demo").exists());
    assert!(!h.data_dir.join("tmp/demo-raw-url.tar.gz").exists());
    assert!(uv_calls(&h.uv_log).is_empty());
    assert!(h.db_row("demo").await.is_none());
}

#[tokio::test]
async fn archive_digest_mismatch_is_fatal_and_removes_the_download() {
    let _guard = ENV_LOCK.lock().await;
    let mut h = Harness::new("digest", false).await;
    let archive = standalone_archive(SERVER_PY);
    let url = h.archive_url("demo.tar.gz");
    // Signed for one archive, served another of the same length: the
    // announced size passes, the digest does not.
    let entry = h
        .hub
        .entry("demo", "", "1.0.0", SERVER_PY, &archive, &url, None, &[]);
    let mut substituted = archive.clone();
    let last = substituted.len() - 1;
    substituted[last] ^= 0xff;
    h.serve_archive("demo.tar.gz", substituted).await;

    h.download_and_materialize(&entry).await.unwrap();

    let steps = h.steps();
    assert_eq!(steps.len(), 1, "{steps:?}");
    assert!(
        steps[0].starts_with("error:download:fatal:sha256 mismatch:"),
        "{}",
        steps[0]
    );
    assert!(!h.data_dir.join("tmp/demo-raw-url.tar.gz").exists());
    assert!(!h.servers_dir().join("demo").exists());
    assert!(h.db_row("demo").await.is_none());
}

#[tokio::test]
async fn signed_length_mismatch_is_refused_before_streaming() {
    let _guard = ENV_LOCK.lock().await;
    let mut h = Harness::new("length", false).await;
    let archive = standalone_archive(SERVER_PY);
    let url = h.archive_url("demo.tar.gz");
    let entry = h
        .hub
        .entry("demo", "", "1.0.0", SERVER_PY, &archive, &url, None, &[]);
    let mut longer = archive.clone();
    longer.extend_from_slice(b"trailing garbage");
    h.serve_archive("demo.tar.gz", longer).await;

    h.download_and_materialize(&entry).await.unwrap();

    let steps = h.steps();
    assert_eq!(steps.len(), 1, "{steps:?}");
    assert!(
        steps[0].starts_with("error:download:fatal:archive length mismatch:"),
        "{}",
        steps[0]
    );
    // Refused on the announced size: no archive file was ever created.
    assert!(!h.data_dir.join("tmp/demo-raw-url.tar.gz").exists());
    assert!(h.db_row("demo").await.is_none());
}

#[tokio::test]
async fn served_digest_contradicting_the_signed_one_is_fatal() {
    let _guard = ENV_LOCK.lock().await;
    let mut h = Harness::new("contradict", false).await;
    let archive = standalone_archive(SERVER_PY);
    let url = h.archive_url("demo.tar.gz");
    let mut entry = h
        .hub
        .entry("demo", "", "1.0.0", SERVER_PY, &archive, &url, None, &[]);
    if let Some(InstallShape {
        source: SourceSpec::RawUrl(spec),
        ..
    }) = entry.install.as_mut()
    {
        spec.sha256 = Some("0".repeat(64));
    }
    h.serve_archive("demo.tar.gz", archive).await;

    h.download_and_materialize(&entry).await.unwrap();

    let steps = h.steps();
    assert_eq!(steps.len(), 1, "{steps:?}");
    assert!(
        steps[0].starts_with("error:download:fatal:archive digest contradiction:"),
        "{}",
        steps[0]
    );
    assert!(h.db_row("demo").await.is_none());
}

#[tokio::test]
async fn dependency_install_failure_leaves_the_tree_unregistered() {
    let _guard = ENV_LOCK.lock().await;
    let mut h = Harness::new("pipfail", true).await;
    let archive = standalone_archive(SERVER_PY);
    let url = h.archive_url("demo.tar.gz");
    let entry = h
        .hub
        .entry("demo", "", "1.0.0", SERVER_PY, &archive, &url, None, &[]);
    h.serve_archive("demo.tar.gz", archive).await;
    h.serve_catalog(&[&entry]).await;

    h.download_and_materialize(&entry).await.unwrap();

    let steps = h.steps();
    assert_eq!(
        steps[..5],
        [
            "complete:download",
            "start:extract",
            "complete:extract",
            "start:install_deps",
            "install:Demo:installing",
        ]
    );
    // The failure detail carries the prefix exactly once, followed by what
    // `uv` printed.
    assert!(
        steps[5].starts_with(
            "error:install_deps:recoverable:uv pip install failed: simulated dependency failure"
        ),
        "{}",
        steps[5]
    );
    assert_eq!(steps.len(), 6, "{steps:?}");

    // Dependencies are installed against the staged tree, so a failure
    // leaves nothing under the servers root, nothing in tmp, and no row —
    // and the catalog view does not mistake a half-built tree for an
    // install.
    assert!(!h.servers_dir().join("demo").exists());
    assert!(!h.data_dir.join("tmp/demo-raw-url.tar.gz").exists());
    assert_eq!(
        std::fs::read_dir(h.data_dir.join("tmp")).unwrap().count(),
        0
    );
    assert!(h.db_row("demo").await.is_none());
    assert_eq!(
        h.catalog_state("demo").await,
        serde_json::json!({
            "installed": false,
            "installed_version": null,
            "update_available": false,
            "running": false,
        })
    );
}

#[tokio::test]
async fn invalid_hub_signature_blocks_registration_after_materialization() {
    let _guard = ENV_LOCK.lock().await;
    let mut h = Harness::new("tamper", false).await;
    let archive = standalone_archive(SERVER_PY);
    let url = h.archive_url("demo.tar.gz");
    let mut entry = h
        .hub
        .entry("demo", "", "1.0.0", SERVER_PY, &archive, &url, None, &[]);
    // The signed identity says 1.0.0; the served entry says otherwise.
    entry.version = "1.0.1".into();
    h.serve_archive("demo.tar.gz", archive).await;

    h.download_and_materialize(&entry).await.unwrap();

    let steps = h.steps();
    let last = steps.last().unwrap();
    assert!(
        last.starts_with("error:finalize:fatal:Ed25519 seal verification failed for 'demo'"),
        "{last}"
    );
    assert!(steps.contains(&"complete:install_deps".to_string()));
    // The verdict is decided on the staged tree: a tamper suspect never
    // reaches the servers root and leaves nothing behind in tmp.
    assert!(!h.servers_dir().join("demo").exists());
    assert_eq!(
        std::fs::read_dir(h.data_dir.join("tmp")).unwrap().count(),
        0
    );
    assert!(h.db_row("demo").await.is_none());
}

#[tokio::test]
async fn unverifiable_signature_registers_unsealed_rather_than_blocking() {
    let _guard = ENV_LOCK.lock().await;
    let mut h = Harness::new("nojwks", false).await;
    let archive = standalone_archive(SERVER_PY);
    let url = h.archive_url("demo.tar.gz");
    let entry = h
        .hub
        .entry("demo", "", "1.0.0", SERVER_PY, &archive, &url, None, &[]);
    // The hub key is unreachable: the JWKS route answers 503.
    h.mock.reset().await;
    h.serve_archive("demo.tar.gz", archive).await;
    Mock::given(method("GET"))
        .and(path("/api/seal/keys"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&h.mock)
        .await;

    h.download_and_materialize(&entry).await.unwrap();

    assert!(h.steps().contains(&"complete:finalize".to_string()));
    let row = h.db_row("demo").await.expect("row");
    assert_eq!(row.seal, None, "registered unsealed");
    assert_eq!(row.installed_version.as_deref(), Some("1.0.0"));
}

// ── the guard the full path enforces ─────────────────────────────────

#[tokio::test]
async fn run_install_refuses_loopback_download_targets() {
    let _guard = ENV_LOCK.lock().await;
    let mut h = Harness::new("loopback", false).await;
    let archive = standalone_archive(SERVER_PY);
    // A local server is exactly what the SSRF guard exists to refuse.
    let url = h.archive_url("demo.tar.gz");
    let entry = h
        .hub
        .entry("demo", "", "1.0.0", SERVER_PY, &archive, &url, None, &[]);
    h.serve_archive("demo.tar.gz", archive).await;

    run_install(&h.state, &entry, HashMap::new(), false)
        .await
        .unwrap();

    let steps = h.steps();
    assert_eq!(
        steps[..4],
        [
            "start:check_installer",
            "complete:check_installer",
            "start:check_uv",
            "complete:check_uv"
        ],
        "the install engine and toolchain checks precede the download: {steps:?}"
    );
    assert_eq!(steps[4], "start:download");
    assert!(
        steps[5].starts_with("error:download:fatal:Access to host '127.0.0.1' is denied"),
        "{}",
        steps[5]
    );
    assert_eq!(steps.len(), 6, "{steps:?}");
    assert!(!h.data_dir.join("tmp/demo-raw-url.tar.gz").exists());
    assert!(!h.servers_dir().join("demo").exists());
    assert!(uv_calls(&h.uv_log).is_empty());
    assert!(h.db_row("demo").await.is_none());
}

// ── the install engine itself: absent or stale is an error, never a fallback ──

#[tokio::test]
async fn run_install_stops_when_the_install_engine_is_missing() {
    let _guard = ENV_LOCK.lock().await;
    let mut h = Harness::new("noengine", false).await;
    let archive = standalone_archive(SERVER_PY);
    let url = h.archive_url("demo.tar.gz");
    let entry = h
        .hub
        .entry("demo", "", "1.0.0", SERVER_PY, &archive, &url, None, &[]);
    h.serve_archive("demo.tar.gz", archive).await;
    let nowhere = h.data_dir.join("no-such-installer");
    std::env::set_var(installer::ENV_OVERRIDE, &nowhere);

    run_install(&h.state, &entry, HashMap::new(), false)
        .await
        .unwrap();

    // Nothing is provisioned, fetched or written: the check comes first
    // and its failure is final.
    let steps = h.steps();
    assert_eq!(steps[0], "start:check_installer");
    assert!(
        steps[1].starts_with(&format!(
            "error:check_installer:fatal:marketplace install engine not found at {}",
            nowhere.display()
        )),
        "{}",
        steps[1]
    );
    assert_eq!(steps.len(), 2, "{steps:?}");
    assert!(uv_calls(&h.uv_log).is_empty());
    assert!(!h.data_dir.join("tmp/demo-raw-url.tar.gz").exists());
    assert!(!h.servers_dir().join("demo").exists());
    assert!(h.db_row("demo").await.is_none());

    // The health endpoint sees the same answer.
    let status = installer::last_status().expect("probed");
    assert_eq!(status.state, InstallerState::Missing);
    assert_eq!(status.path, nowhere);
}

#[tokio::test]
async fn run_install_stops_when_the_install_engine_is_another_version() {
    let _guard = ENV_LOCK.lock().await;
    let mut h = Harness::new("staleengine", false).await;
    let archive = standalone_archive(SERVER_PY);
    let url = h.archive_url("demo.tar.gz");
    let entry = h
        .hub
        .entry("demo", "", "1.0.0", SERVER_PY, &archive, &url, None, &[]);
    h.serve_archive("demo.tar.gz", archive).await;

    // An engine left behind by another release: it runs and identifies
    // itself, but not as the version this kernel was built with.
    let stale = h.data_dir.join("stale-installer");
    std::fs::write(
        &stale,
        "#!/bin/sh\necho 'cloto-installer 0.0.0 commit=none go=go0 test/arch'\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&stale, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var(installer::ENV_OVERRIDE, &stale);

    run_install(&h.state, &entry, HashMap::new(), false)
        .await
        .unwrap();

    let steps = h.steps();
    assert_eq!(steps[0], "start:check_installer");
    assert!(
        steps[1].starts_with("error:check_installer:fatal:marketplace install engine at ")
            && steps[1].contains("is version 0.0.0, this ClotoCore is "),
        "{}",
        steps[1]
    );
    assert_eq!(steps.len(), 2, "{steps:?}");
    assert!(uv_calls(&h.uv_log).is_empty());
    assert!(h.db_row("demo").await.is_none());

    let status = installer::last_status().expect("probed");
    assert_eq!(status.state, InstallerState::VersionMismatch);
    assert_eq!(status.version.as_deref(), Some("0.0.0"));
    assert_eq!(status.expected, env!("CARGO_PKG_VERSION"));
}

// ── The response answers whether the install started ──────────────────
//
// `install_handler` spawns the work and returns immediately, so its body is
// the only thing a caller that does not subscribe to the progress stream ever
// sees. These pin that the body is not `{"started": true}` when the install
// could not start.

/// Drive the handler the way an operator's script does, and read back what
/// that script would read: the status line and the body. Asserting on the
/// internal error value instead would not say whether a caller can tell the
/// two outcomes apart, which is the whole question here.
async fn post_install(
    h: &Harness,
    server_id: &str,
    update: bool,
) -> (axum::http::StatusCode, serde_json::Value) {
    post_install_starting(h, server_id, update, false).await
}

/// `post_install` with control over `auto_start`, for the tests that need the
/// server to be running before the request under test.
async fn post_install_starting(
    h: &Harness,
    server_id: &str,
    update: bool,
    auto_start: bool,
) -> (axum::http::StatusCode, serde_json::Value) {
    use axum::response::IntoResponse;

    let mut headers = HeaderMap::new();
    headers.insert("X-API-Key", API_KEY.parse().unwrap());
    let outcome = install_handler(
        ConnectInfo("127.0.0.1:9999".parse().unwrap()),
        State(h.state.clone()),
        headers,
        axum::Json(InstallRequest {
            server_id: server_id.to_string(),
            env: None,
            auto_start: Some(auto_start),
            update: Some(update),
        }),
    )
    .await;
    let response = match outcome {
        Ok(json) => json.into_response(),
        Err(err) => err.into_response(),
    };
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

#[tokio::test]
async fn install_refuses_up_front_when_the_engine_is_another_version() {
    let _guard = ENV_LOCK.lock().await;
    let h = Harness::new("handlerstale", false).await;
    let archive = standalone_archive(SERVER_PY);
    let url = h.archive_url("demo.tar.gz");
    let entry = h
        .hub
        .entry("demo", "", "1.0.0", SERVER_PY, &archive, &url, None, &[]);
    h.serve_catalog(&[&entry]).await;

    let stale = h.data_dir.join("stale-installer");
    std::fs::write(
        &stale,
        "#!/bin/sh\necho 'cloto-installer 0.0.0 commit=none go=go0 test/arch'\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&stale, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var(installer::ENV_OVERRIDE, &stale);

    let (status, body) = post_install(&h, "demo", false).await;
    assert_eq!(
        status,
        axum::http::StatusCode::CONFLICT,
        "a stale engine cannot install, and the status has to say so: {body}"
    );
    assert_ne!(
        body.pointer("/data/started"),
        Some(&serde_json::json!(true)),
        "the body must not read as a started install: {body}"
    );
    let message = body.to_string();
    assert!(
        message.contains("is version 0.0.0") && message.contains("this ClotoCore is "),
        "the refusal has to name the fault an operator resolves: {message}"
    );

    // Refused, not merely reported: nothing ran and nothing was recorded.
    assert!(uv_calls(&h.uv_log).is_empty());
    assert!(h.db_row("demo").await.is_none());
    // The concurrency flag is free for the next request — a refusal that took
    // it would wedge every later install behind "already in progress".
    assert!(!h
        .state
        .setup_in_progress
        .load(std::sync::atomic::Ordering::SeqCst));

    std::env::set_var(installer::ENV_OVERRIDE, install_engine());
}

#[tokio::test]
async fn install_still_starts_when_the_engine_is_the_matching_version() {
    let _guard = ENV_LOCK.lock().await;
    let h = Harness::new("handlerok", false).await;
    let archive = standalone_archive(SERVER_PY);
    let url = h.archive_url("demo.tar.gz");
    let entry = h
        .hub
        .entry("demo", "", "1.0.0", SERVER_PY, &archive, &url, None, &[]);
    h.serve_catalog(&[&entry]).await;
    h.serve_archive("demo.tar.gz", archive).await;
    std::env::set_var(installer::ENV_OVERRIDE, install_engine());

    let (status, body) = post_install(&h, "demo", false).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
    assert_eq!(
        body.pointer("/data/started"),
        Some(&serde_json::json!(true))
    );

    // The spawned task owns `setup_in_progress`; let it finish so it does not
    // leak into the next test through the shared engine override.
    let handle = h.state.install_task.lock().await.take();
    if let Some(handle) = handle {
        let _ = handle.await;
    }
}

// ── a failed update and the server it stopped ────────────────────────────

/// The status as the manager reports it, by name — `Error` carries a message
/// that differs between two failed connects, and the question here is which
/// state the server is in, not what the last connect said.
async fn server_state(h: &Harness, id: &str) -> String {
    let statuses = h.state.mcp_manager.registered_server_statuses().await;
    match statuses.get(id) {
        Some(status) => serde_json::to_value(status)
            .expect("status serializes")
            .as_str()
            .expect("status serializes to a name")
            .to_string(),
        None => "absent".to_string(),
    }
}

/// Install `demo` for real and leave it registered and started, then hand back
/// the entry the update will be asked for.
///
/// Address-pinned, so the download guard does not refuse the local server, and
/// registered *unsealed* (the hub key is unreachable) so that starting it is
/// decided by the connect and not by the seal. A tree seal is verified against
/// `sandbox_base_dir.parent()/mcp-servers`, and this crate's test `AppState`
/// never calls `configure_isolation`, so the manager keeps its default relative
/// sandbox path and cannot find the tree it just installed. Production sets it
/// (`lib.rs`), so that divergence belongs to the harness — but it would
/// otherwise decide the outcome of these tests, which are about the update.
async fn installed_and_started(h: &Harness) -> RegistryEntry {
    let archive = standalone_archive(SERVER_PY);
    let url = h.archive_url("demo.tar.gz");
    let entry = h
        .hub
        .entry("demo", "", "1.0.0", SERVER_PY, &archive, &url, None, &[]);
    h.mock.reset().await;
    h.serve_archive("demo.tar.gz", archive).await;
    Mock::given(method("GET"))
        .and(path("/api/seal/keys"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&h.mock)
        .await;
    h.serve_catalog(&[&entry]).await;
    std::env::set_var(installer::ENV_OVERRIDE, install_engine());
    let outcome = h
        .download_and_materialize_starting(&entry, true)
        .await
        .expect("the first install runs");
    assert_eq!(
        outcome,
        InstallOutcome::Installed,
        "the test needs a real install to update"
    );
    entry
}

/// Drive an update that cannot land — through the handler, so it takes the
/// same stop-then-re-vendor path an operator's Update button does. The
/// download stage refuses the loopback address the archive is served from,
/// which is a failure before anything touches the installed tree.
async fn failed_update(h: &Harness) {
    let (status, body) = post_install_starting(h, "demo", true, true).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
    let handle = h.state.install_task.lock().await.take();
    if let Some(handle) = handle {
        let _ = handle.await;
    }
}

#[tokio::test]
async fn a_failed_update_leaves_the_server_it_stopped_in_the_state_it_found_it() {
    let _guard = ENV_LOCK.lock().await;
    let h = Harness::new("updfail", false).await;
    let entry = installed_and_started(&h).await;

    let before = server_state(&h, "demo").await;
    assert_ne!(
        before, "Disconnected",
        "the test measures nothing unless the update has a running server to stop"
    );
    assert_ne!(
        before, "absent",
        "the install must have registered a handle"
    );

    failed_update(&h).await;

    // The tree the server runs from is still the one the failed update never
    // replaced, so there is something to run.
    assert!(
        h.data_dir.join("mcp-servers/demo").is_dir(),
        "a failure before the swap leaves the installed tree in place"
    );
    assert_eq!(
        server_state(&h, "demo").await,
        before,
        "a failed update must not leave the connector stopped"
    );
    assert!(
        h.db_row(&entry.id).await.is_some(),
        "the row the update preserved is still there"
    );
}

#[tokio::test]
async fn a_failed_update_leaves_a_stopped_server_stopped() {
    let _guard = ENV_LOCK.lock().await;
    let h = Harness::new("updfailstopped", false).await;
    installed_and_started(&h).await;
    h.state
        .mcp_manager
        .stop_server("demo")
        .await
        .expect("the operator stops the server");
    assert_eq!(server_state(&h, "demo").await, "Disconnected");

    failed_update(&h).await;

    // The restart undoes this update's own stop. A server the operator had
    // already stopped was not stopped by the update, so starting it here would
    // be the update deciding to run something the operator had shut down.
    assert_eq!(
        server_state(&h, "demo").await,
        "Disconnected",
        "a failed update must not start a server the operator had stopped"
    );
}

#[tokio::test]
async fn a_failed_install_records_no_install_receipt() {
    let _guard = ENV_LOCK.lock().await;
    let h = Harness::new("noreceipt", false).await;
    let archive = standalone_archive(SERVER_PY);
    let url = h.archive_url("demo.tar.gz");
    let entry = h
        .hub
        .entry("demo", "", "1.0.0", SERVER_PY, &archive, &url, None, &[]);
    h.mock.reset().await;
    h.serve_catalog(&[&entry]).await;
    std::env::set_var(installer::ENV_OVERRIDE, install_engine());

    // Fails at the download guard, which refuses the loopback address the
    // archive is served from — before anything is written under the data dir.
    let (status, body) = post_install(&h, "demo", false).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
    let handle = h.state.install_task.lock().await.take();
    if let Some(handle) = handle {
        let _ = handle.await;
    }
    assert!(h.db_row("demo").await.is_none(), "nothing was installed");

    let listed = cloto_core::defender::footprint::load(&h.data_dir).is_some_and(|receipt| {
        receipt
            .entries
            .iter()
            .any(|candidate| candidate.id == "mcp:demo")
    });
    assert!(
        !listed,
        "the ledger the defender treats as canonical must not carry a directory that was never created"
    );
}

#[tokio::test]
async fn a_failed_update_keeps_what_the_proxy_learned_about_its_callers() {
    let _guard = ENV_LOCK.lock().await;
    let h = Harness::new("updevidence", false).await;
    installed_and_started(&h).await;

    // The record a served proxy request leaves. The proxy writes it from
    // inside its own module and exposes no writer, so the row is seeded
    // directly; the assertion below reads it back through the public reader.
    // HARDCODED(managers/llm_proxy.rs::KERNEL_STORE_ID, ::TOKEN_EVIDENCE_KEY):
    // the store id and key are private to that module, and a test that seeds
    // the row it reads back has to name it.
    sqlx::query("INSERT OR REPLACE INTO plugin_data (plugin_id, key, value) VALUES (?, ?, ?)")
        .bind("cloto.kernel")
        .bind("llm_proxy.token_evidence")
        .bind(r#"{"served":true,"untrusted":true}"#)
        .execute(&h.state.pool)
        .await
        .expect("seed the proxy's record");

    failed_update(&h).await;

    // Clearing it is what an install earns by replacing the files the record
    // describes. This update replaced nothing, so the record still describes
    // the connector that is running.
    assert_eq!(
        cloto_core::managers::llm_proxy::load_token_evidence(&h.state.pool).await,
        cloto_core::managers::llm_proxy::TokenEvidence {
            served: true,
            untrusted: true
        },
        "a failed update must not clear what the proxy learned"
    );
}

#[tokio::test]
async fn a_failed_update_with_no_tree_left_does_not_start_the_server() {
    let _guard = ENV_LOCK.lock().await;
    let h = Harness::new("updnotree", false).await;
    installed_and_started(&h).await;
    assert_ne!(server_state(&h, "demo").await, "Disconnected");

    // The one failure that can take the installed tree with it is the swap
    // itself: the engine removes the old tree and renames the staged one in,
    // and a rename that fails leaves neither. Reproduced here by removing the
    // tree, because what the restart has to answer is "is there anything to
    // run", not "which step failed".
    std::fs::remove_dir_all(h.servers_dir().join("demo")).expect("remove the installed tree");

    failed_update(&h).await;

    assert_eq!(
        server_state(&h, "demo").await,
        "Disconnected",
        "with no tree on disk there is nothing to start, and trying says the opposite in the log"
    );
}
