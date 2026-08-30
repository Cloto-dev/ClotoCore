//! Characterization tests for the marketplace `raw_url` install path.
//!
//! These pin what the installer *does today* at the boundaries another
//! implementation would have to reproduce: what is fetched and how it is
//! verified, where the tree lands on disk, which commands the Python
//! environment step runs, what each failure leaves behind, and — above
//! all — what is written to the database and when. They record current
//! behaviour, including quirks; a change in behaviour belongs in its own
//! commit with the expectation updated first.
//!
//! The download stage refuses loopback addresses (its SSRF guard), so the
//! whole path cannot be driven end to end against a local server. The
//! tests drive the two stages `install_from_raw_url` is made of —
//! `download_raw_url_archive` with an unpinned client against a local
//! server, then `materialize_and_register` — exactly as the production
//! wrapper chains them, and pin the guard itself through `run_install`.
//! The Python environment step is observed through a stand-in `uv` that
//! records its arguments, which is why this file is Unix-only.
#![cfg(unix)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use cloto_core::handlers::marketplace::{
    catalog_handler, download_raw_url_archive, materialize_and_register, run_install, CatalogQuery,
    RegistryEntry,
};
use cloto_core::handlers::setup::SetupProgressEvent;
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

/// A gzipped tarball with the given entries, GitHub-style (one shared
/// top-level directory, which the installer strips).
fn tarball(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut tar_buf = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_buf);
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
    /// with an unpinned HTTP client so the local server is reachable.
    async fn download_and_materialize(&self, entry: &RegistryEntry) -> anyhow::Result<()> {
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
        let client = reqwest::Client::new();
        if !download_raw_url_archive(
            &self.state.setup_progress_tx,
            entry,
            spec,
            client,
            &archive_path,
        )
        .await?
        {
            return Ok(());
        }
        materialize_and_register(
            &self.state,
            entry,
            spec.subdir.as_deref(),
            &archive_path,
            &tmp_dir,
            HashMap::new(),
            false,
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
        cloto_core::managers::mcp_venv::resolve_venv_dir()
            .unwrap_or_else(|| self.servers_dir().join(".venv"))
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
    // does not exist yet, then install the server tree into it.
    let venv = h.venv_dir();
    let mut expected_uv = Vec::new();
    if !venv_before {
        expected_uv.push(format!("venv --python 3.13 {}", venv.display()));
    }
    expected_uv.push(format!(
        "pip install --no-progress --python {} {}",
        venv.join("bin/python").display(),
        install_dir.display()
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

    // `common` is installed into the venv before the connector itself.
    let venv = h.venv_dir();
    let python = venv.join("bin/python").display().to_string();
    let calls = uv_calls(&h.uv_log);
    let pip_calls: Vec<&String> = calls.iter().filter(|c| c.starts_with("pip ")).collect();
    assert_eq!(
        pip_calls,
        [
            &format!(
                "pip install --no-progress --python {python} {}",
                install_dir.join("servers/common").display()
            ),
            &format!(
                "pip install --no-progress --python {python} {}",
                server_path.display()
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
    // The failure detail is wrapped twice ("uv pip install failed: uv pip
    // install failed: ..."): recorded as it is, not endorsed.
    assert!(
        steps[5].starts_with(
            "error:install_deps:recoverable:uv pip install failed: uv pip install failed: "
        ),
        "{}",
        steps[5]
    );
    assert_eq!(steps.len(), 6, "{steps:?}");

    // The extracted tree stays on disk, the archive is cleaned up, and no
    // row is written. The catalog view then reports the server as installed
    // on the strength of the directory alone, with no version — recorded
    // here as current behaviour, not endorsed.
    assert!(h.servers_dir().join("demo/server.py").is_file());
    assert!(!h.data_dir.join("tmp/demo-raw-url.tar.gz").exists());
    assert!(h.db_row("demo").await.is_none());
    assert_eq!(
        h.catalog_state("demo").await,
        serde_json::json!({
            "installed": true,
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
    assert!(h.servers_dir().join("demo/server.py").is_file());
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
        steps[..2],
        ["start:check_uv", "complete:check_uv"],
        "the toolchain check precedes the download: {steps:?}"
    );
    assert_eq!(steps[2], "start:download");
    assert!(
        steps[3].starts_with("error:download:fatal:Access to host '127.0.0.1' is denied"),
        "{}",
        steps[3]
    );
    assert_eq!(steps.len(), 4, "{steps:?}");
    assert!(!h.data_dir.join("tmp/demo-raw-url.tar.gz").exists());
    assert!(!h.servers_dir().join("demo").exists());
    assert!(uv_calls(&h.uv_log).is_empty());
    assert!(h.db_row("demo").await.is_none());
}
