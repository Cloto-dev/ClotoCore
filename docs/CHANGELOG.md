# Changelog

All notable changes to ClotoCore are documented in this file.

Format follows [Keep a Changelog](https://keepachangelog.com/).
Versioning follows the project's phase scheme: Alpha (A), Beta (βX.Y = 0.X.Y), Stable (1.X.Y).

---

## [0.6.8-beta.6] — 2026-09-03

Soak release. It is the first cut that carries the separate install engine, and
this line counts soak on the *published* artifact — an engine that only ever ran
from a developer checkout has not been soaked at all.

### Added

- **Marketplace installs from a raw URL run through a separate install engine.**
  `cloto-installer` is a single binary shipped beside the application; it
  fetches the archive, verifies the seal, extracts it and builds the virtual
  environment, and the kernel records the resulting registration. When the
  engine is absent, or reports a version other than the one this build expects,
  the install stops with a message naming both versions instead of falling back
  to a second implementation — a missing engine is a visible failure, never a
  silent change of behaviour. `GET /api/health/scan` reports the engine's state
  under `installer`. (#469, #470, #471, #472)
- **A published documentation site**, built from `docs/` and gated: the build
  is strict, every in-site link and heading anchor must resolve, and the
  measurable claims documents make about the project — test counts, the current
  version, environment defaults — are checked against the code on every run.
  (#476, #477, #478)

### Fixed

- **Deleting a dynamic MCP server deletes its generated script** (bug-505). The
  three paths that create such a server wrote its Python into the directory
  named by `MCP_SCRIPTS_DIR`, while deletion looked under a legacy `scripts/`
  directory the writers had stopped using. The existence check was therefore
  always false, the unlink was never reached, and the user-supplied Python
  stayed on disk after the server and its connection were gone. The location is
  now derived in one place that every call site goes through, and a removal
  failure is logged with its path rather than discarded. (#463)
- **The Windows dependency tree is unified again.** An `h2` bump had split
  `windows-sys` into two versions in the same build. (#460)
- Lint and Security Audit are green on `master` again. (#459)

### Changed

- The repository's published text is English. (#461)
- The verification harness requires its host configuration to be supplied
  rather than shipping a default that pointed at one particular machine. (#458)

---

## [0.6.8-beta.5] — 2026-08-09

Soak release, cut because the first fix below is a CRITICAL one that beta.4
does not contain — and this line counts soak on the *published* artifact, so
leaving it on master would mean two weeks of soak that never exercised it.

### Fixed

- **Uninstalling actually uninstalls** (bug-499, CRITICAL). The Danger Zone's
  full uninstall removed nothing and left the window on a shutdown overlay
  that never resolved. `POST /api/system/uninstall` staged the purge plan,
  copied the helper, launched it and signalled the kernel's shutdown — which
  stopped the HTTP server and nothing else. The detached helper waits for its
  parent to exit and, when it does not, bails without touching anything, which
  is correct on its own terms: deleting a running installation is how targets
  end up half-removed. The app now ends its process when the kernel says
  shutdown. `POST /api/system/shutdown` shared the same defect silently and is
  fixed with it. Verified end to end on a real installed build, not only in a
  test. (#432)
- **Escape closes a dialog** (bug-501). No modal in the dashboard could be
  dismissed with the key every desktop application answers to — including the
  settings modal, whose close control also carried no accessible name, so a
  keyboard or screen-reader user had no way out but the backdrop. (#444)
- **The window controls say what they are** (bug-502). Minimise, maximise,
  close and the modal dismiss buttons were icon-only with no accessible name,
  reaching assistive technology as unlabelled buttons. (#441, #444)

### Added

- The environment may pass extra WebView2 browser arguments at launch
  (`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`), which is what lets the
  verification harness reach the DOM of a *shipped* build rather than an
  instrumented one. Closed unless the variable is set, so a normal launch is
  unchanged. (#433)

---

## [0.6.8-beta.4] — 2026-08-03

Soak release. It exists because the fix below is not in beta.3, and the
promotion criteria for this line count soak time on the *published* artifact —
not on whatever master happens to hold.

### Fixed

- **The chat pane follows new turns again** (bug-498). Its scroll effect had
  been reduced to a mount-only effect since 2026-03-15, when a lint autofix
  removed the dependencies it used as change triggers, so a reply rendered
  below the fold and the view never moved — indistinguishable, from the user's
  side, from a reply that never arrived. Every release since had shipped that
  way. Pinning now lives in a `useStickToBottom` hook driven by a
  MutationObserver, which also follows a reply as the typewriter reveals it.
  Found by the opverify visual apex against the real installed GUI, and
  re-verified there after the fix. (#429)

### Changed

- Dependency bumps (patch/minor only): tokio 1.53.1, tauri 2.11.5,
  rust-embed 8.12.0, log 0.4.33; `@tauri-apps/cli` 2.11.4, `@biomejs/biome`
  2.5.6, `@tauri-apps/plugin-global-shortcut` 2.3.2.
  (#421, #425, #426, #427, #422, #423, #424)

> Note: 0.6.8-beta.3 (2026-08-03) shipped without an entry here; its contents
> are in the GitHub release notes.

---

## [0.6.8-beta.2] — 2026-07-12

Second beta of the 0.6.8 line — and the **first release whose updater
signatures verify against the shipped pubkey** (see Fixed): desktop
auto-update works from this release onward. Also the first release published
through the signed updater feed (`updater-feed` release: `manifest.json` +
`stable`/`current`/`experimental` channel views). **Installs of v0.6.8-beta.1
or older must update manually once** — their embedded key cannot verify any
signature ever published.

### Added

- **Consensus revived** as in-kernel orchestration: answers are
  delivered to the originating agent's chat plus a dedicated Consensus tab,
  engine-reuse quorum fallback (`CONSENSUS_ENGINE_REUSE`), per-agent engine
  access enforcement, and long-term memory storage of verdicts.
  (#230–#232, #237, #242, #266)
- **Safe shutdown**: tray quit and the new sidebar power button
  share one drain-then-exit sequence with a shutdown overlay; MCP subprocess
  trees are reaped on restart and app exit, with a forced group-kill sweep
  when the drain window expires (bug-426). (#248, #264, #265)
- **Per-agent recall configuration**: recall timing policy,
  session scope with the per-channel episode axis (default flipped after
  A/B), and read-edit-save precision control in the dashboard. (#190–#203)
- **Per-server MCP log streaming** in the dashboard Log tab. (#244)
- **LLM provider metadata** seeded from the marketplace catalog; the agent
  console lists only real engines and warns on uninstall.
  (#249–#251)
- **Release pipeline**: Release Lifecycle Standard adoption
  (`SUPPORT.md` / `SECURITY.md`), signed updater feed with per-tier channel
  views, certification recorded in `.release/lifecycle.json`. (#269–#271)

### Fixed

This beta closes **33 registry-tracked bugs** (2 critical / 13 high /
14 medium / 4 low) plus the updater key repair — the largest fix set of any
0.6.x release. Itemized below by area; `qa/issue-registry.json` is the
verification source of truth for every entry.

- **Updater signing key mismatch (CRITICAL, latent since v0.6.0)**: the
  2026-03-08 key rotation updated the client-embedded pubkey but not the CI
  signing secret, so every published artifact was signed with a key no
  shipped client could verify — `downloadAndInstall()` always failed
  signature verification (update *notifications* worked, masking it).
  Rotated to a fresh pair (`66FD7C5172819DEC`); key-management runbook added
  to the pipeline design doc. (#272)

**Security audit sweep** (PRs #204/#209/#227/#237/#240):

- **bug-400** (CRITICAL): marketplace uninstall was vulnerable to path
  traversal — `DELETE /api/marketplace/servers/:id` pushed the unvalidated
  `server_id` into the on-disk path, allowing arbitrary directory deletion.
  (#209)
- **bug-415** (CRITICAL): the first bug-412 fix could poison the SQLite pool
  when an audit write was cancelled — a regression caught pre-merge by the
  post-fix adversarial review; never shipped. (#227)
- **bug-403 / bug-407 / bug-414** (HIGH, SSRF hardening): the
  restricted-address check missed IPv6 link-local and IPv4-mapped addresses
  (403); the validated IP was discarded after the check, leaving a
  DNS-rebinding TOCTOU — the connection is now pinned to the validated
  address (407); redirects could escape the whitelist + IP pin — every hop
  is now re-validated (414). (#209, #227)
- **bug-406 / bug-420 / bug-421** (HIGH, per-agent access enforcement): the
  `tool_hint` direct-execution path allowed `agent_id` spoofing (406) and
  bypassed per-agent MCP access control entirely — an I/O bridge could
  invoke tools on servers the agent was never granted (420); enforcement was
  fragmented across call sites and reasoning *engines* were never
  access-checked at all — now unified behind a single capability gate
  covering both tools and engines (421). (#209, #237, #240)
- **bug-409 / bug-410** (MEDIUM, resource exhaustion): unbounded allocation
  in `StreamAssembler::record_chunk` on an attacker-controlled chunk index
  (409); unbounded read buffer in `parse_sse_stream` when a server streams
  a large body with no newline (410). (#209)

**Kernel stability & correctness:**

- **bug-401 / bug-402** (HIGH/MEDIUM): UTF-8 char-boundary panics — byte
  slicing of log/error strings crashed on multibyte codepoints in the MCP
  stdio response loop (401) and `parse_chat_think_result` error paths
  (402). (#209)
- **bug-405** (MEDIUM): cron intervals had no upper bound — an oversized
  value overflowed `i64` and produced runaway perpetual dispatch. (#209)
- **bug-411 / bug-412** (MEDIUM): `McpClient::is_alive()` inferred liveness
  from the write channel, so an idle child that died was never auto-restarted
  by the health monitor (411); the audit-log writer's DEFERRED read-then-write
  transaction hit un-retried `SQLITE_BUSY_SNAPSHOT` under concurrency (412).
  (#227)
- **bug-288** (HIGH): the `delete_agent_data` memory-plugin tool name was
  hard-coded in the kernel. (#204)
- **bug-395** (HIGH): `sandbox_base_dir` — and therefore the Magic Seal key
  directory — defaulted to a CWD-relative path instead of anchoring to the
  absolute data dir. (#184)

**MCP server management & marketplace:**

- **bug-396 / bug-397 / bug-398**: `Agent.default_engine_id`
  `mind.deepseek` was unresolvable — engine resolution requires an exact
  server-name match (396, HIGH); `GET /api/mcp/servers/:name/settings`
  returned 500 for every server — the SELECT omitted the `seal` column
  (397, HIGH); interpreter-launched servers were spawned without verifying
  the entry-point script exists, so stale paths failed opaquely (398,
  MEDIUM). (#204)
- **bug-399 / bug-404** (HIGH): monorepo connectors were installed to a
  doubled on-disk path and failed to launch (399, #207); the MCP venv
  resolver anchored to `exe_dir()/data` instead of `config::data_dir()`, so
  installed builds couldn't find marketplace-created venvs (404, #209).
- **In-place marketplace update** for installed servers preserves grants +
  env instead of destroy-and-reinstall. (#198)
- **Orphan-process class closed from both sides**: MCP subprocess *trees*
  are reaped on server restart and app exit (#248); `drain_all`'s global
  timeout cancelled in-flight kills between SIGTERM and SIGKILL, leaking
  survivors past app exit — per-server drains are now detached tasks with a
  forced group-kill sweep on timeout (bug-426, MEDIUM, #265). The
  server-side half (mgp-discord exiting on stdin EOF) is clotohub-servers
  bug-009.

**Consensus (revival hardening):**

- **bug-417** (HIGH): the terminal consensus response was never delivered to
  the originating chat — a silent hang from the requester's perspective.
  (#231)
- **bug-408** (MEDIUM): a straggler proposal arriving during synthesis could
  be mistaken for the synthesizer's output — eliminated structurally by the
  event-driven in-kernel redesign. (#230)
- **bug-419** (MEDIUM): the global `CONSENSUS_ENGINES` list ran on every
  agent, executing engines the requesting agent was never assigned. (#237)
- **bug-422** (MEDIUM): consensus verdicts were persisted to chat history
  but never written to the agent's long-term memory, so later turns couldn't
  recall them. (#242)
- **bug-418** (LOW): the shipped `.env` example referenced an engine that is
  not auto-registered, so consensus failed out of the box for anyone
  uncommenting it. (#266)

**Dashboard:**

- **bug-413 / bug-416**: MemoryCore fetched a *global* most-recent top-N and
  filtered client-side, under-displaying per-agent memories (413, MEDIUM,
  #225); the tab bar rendered an un-scopable empty-string agent tab for
  global-pool rows (416, LOW, #227).
- **bug-425** (MEDIUM): the agent console engine selector filtered grants by
  the `mind.` prefix only, hiding de-prefixed catalog engines. (#245)
- **bug-423 / bug-424** (LOW): the MCP log tab's SSE filter read
  `event.payload.*` where kernel events carry `event.data.*`, and matched
  `'MCP'` against mixed-case `Mcp*` discriminants — together the filter was
  never true. (#244)
- Engines/memory classified by tool surface instead of id prefix (#234);
  deleting a passwordless agent no longer sends an empty JSON body (#235);
  recall `PillSelect` popover portaled to `document.body` to escape clipping
  (#200); knob2 v2 store/recall channel symmetry (#201).

**CI:**

- `CREATED_MAP` initialization so the registry-sync no-new-entries path
  survives `set -u` (#268); master unblocked on new clippy 1.97 lints + the
  crossbeam-epoch advisory (#267).

### Changed

- Engine/server ids de-prefixed: the `mind.` engine prefix and category id
  prefixes are retired; classification is tool-surface based. (#247, #252)
- axum 0.8 migration and dependency refresh (tauri 2.11, tower-http,
  quinn-proto RUSTSEC advisory). (#224 et al.)
- Cross-platform release gate: Windows tests + headless `--smoke` in CI. (#185)

---

## [0.6.8-beta.1] — 2026-06-13

**Feature freeze for the 0.6.8 line.** This beta completes the P1/P2 design-violation remediation programme: every remaining `ARCHITECTURE.md` §1 (Design Principles) violation in the kernel is resolved, alongside the Setup Wizard / marketplace robustness, MCP transport-timeout, and lifecycle hardening backlog. `scripts/verify-issues.sh` reports zero open HIGH/MEDIUM entries for that set. Published on the alpha channel (the Tauri Updater resolves `latest.json` via `/releases/latest/download/` and skips prerelease entries, so production `v0.6.7` stable installs receive no in-app upgrade prompt). No installer / upgrade-hook behaviour changes.

### Fixed

- **bug-365 / bug-366 / bug-367 / bug-369** (HIGH, Setup Wizard / marketplace robustness): empty-batch install now early-returns with an i18n error instead of a silent no-op (365); in-flight install tasks are tracked on `AppState` and aborted on shutdown so child `uv`/`pip` processes are reaped via `kill_on_drop` instead of orphaned (366); a sha256 tarball mismatch no longer swallows the `remove_file` error (367); `build_and_register`'s common+server pip install is unified onto the same null-stdio `status()` / streaming-spawn path as batch installs, removing a pipe-buffer deadlock risk and adding a server-install timeout (369). (#178)
- **bug-304 / bug-355 / bug-356 / bug-357 / bug-358** (MEDIUM, MCP transport timeout & resource discipline): stdio writer `write_all`/`flush` gains a 10 s timeout (355); HTTP transport adds a 30 s per-message timeout on top of the global cap (356); every `McpClient` send goes through a 10 s `send_with_timeout` helper that cleans up `pending_requests` on failure (357); process kill escalates SIGTERM → 3 s grace → SIGKILL on Unix (358); the existing `Drop` + `kill_on_drop` resource guarantee is re-anchored and documented (304). (#179)
- **bug-282 / bug-285** (HIGH, §1.2 Capability over Concrete Type): the consensus synthetic agent id (`SYSTEM_CONSENSUS_AGENT` const) and the `"consensus:"` trigger prefix are no longer hard-coded — they are `ConsensusConfig.synthetic_agent_id` (env `CONSENSUS_AGENT_ID`) and `AppConfig.consensus_prefix` (env `CONSENSUS_PREFIX`), both with back-compatible defaults. (#180)
- **bug-293** (HIGH, §1.4 Data Sovereignty): the kernel no longer interprets the `engine_routing` agent metadata. The `RoutingRule` schema, its deserialization, and CFR/fallback evaluation moved from the kernel handler (`handlers/engine_routing.rs`, deleted) into an in-tree plugin (`plugins/routing_default.rs`); the handler now forwards opaque metadata via a direct in-process call with no added latency. (#181)
- **bug-305 / bug-313 / bug-342** (MEDIUM/LOW, lifecycle & window hardening): shutdown drains all MCP servers in a bounded task (5 s/server, 10 s global) before notifying shutdown waiters (305); restart-policy defaults are consolidated into `DEFAULT_MAX_RESTARTS` et al. constants and documented in `ARCHITECTURE.md` §3.1.2 so code and docs cannot drift (313); every fallible Tauri window op (`show`/`unminimize`/`set_focus`/`hide`/`navigate`) routes through a `with_window_log!` macro that logs a warning on failure instead of discarding the `Result`, so WebView2 corruption no longer leaves the backend running headless without a signal (342). (#181)

### Changed

- **`config::data_dir`** — the production user-data directory segment is promoted from a bare `"cloto-system"` string literal to a single named constant `config::APP_DATA_DIR_NAME`. No behavioural change: the value is unchanged and deliberately kept as `cloto-system` (not the current "ClotoCore" branding) because `installer.nsh` and the bug-386 legacy-install detection preserve existing user databases at this exact path; renaming it would orphan installed users' data and require a boot-time migration. (#182)

### Verified

- `scripts/verify-issues.sh`: bug-293 / 342 `[FIXED]` (pattern absent), bug-305 / 313 `[VERIFIED]` (fix-marker present), with the full remediation set (bug-282/285/304/355–358/365–369) confirmed across PRs #178–#181; 0 stale / 0 errors.
- CI green on every PR: clippy (workspace allow-list, `--exclude app`) + fmt + workspace tests (371 passed, incl. 6 new `routing_default` tests) + `cargo check -p app`.

---

## [0.6.8-alpha.4] — 2026-06-13

Marketplace install-chain remediation + catalog seal verification. Fourth prerelease in the 0.6.8 line, published on the alpha channel (the Tauri Updater resolves `latest.json` via `/releases/latest/download/` and skips `prerelease` entries, so production `v0.6.7` stable installs receive no in-app upgrade prompt). This release lands the full ClotoCore-side fix set discovered during the 2026-06-10 live marketplace install debug (bug-388–392) plus the hub-mediated catalog seal verification chain ( bug-394) and the supporting `raw_url` distribution fixes. No installer / upgrade-hook behaviour changes — only kernel marketplace, DB migration, and dashboard paths.

### Fixed

- **bug-391** (CRITICAL): Magic Seal verification hashed `config.command` (`"python"`) instead of the entry-point script. For interpreter-launched servers `Path::new("python").exists()` was false, so verification was skipped via the `entry_point_not_a_file` branch and every Python marketplace server was force-downgraded to Untrusted — the catalog-issued seal was never checked. Fix: resolve the sealable file as `args[0]` when the command is an interpreter (`resolve_sealable_entry_point`, `managers/mcp.rs`). (#167)
- **bug-389 / bug-390** (HIGH): `needs_common` detection probed the legacy flat layout (`servers_dir/common/__init__.py`) which marketplace git-clone installs never populate, and the `uv pip install common` rescue was dead code (it required a `servers/common/pyproject.toml` absent from the monorepo). Connectors declaring `install.dependencies=["common"]` died with `ModuleNotFoundError: No module named 'common'`. Fix: dependency-driven common provisioning for the nested clone layout. (#168, #173)
- **bug-388** (HIGH): migration `20260524010000` (rename `memory.cpersona` → `cpersona`) aborted kernel boot with a PRIMARY KEY collision (`UNIQUE constraint failed: mcp_servers.name`) when both the legacy row and a marketplace-installed `cpersona` row existed (any pre-0.6.6 user who installed cpersona from the catalog before first booting a ≥0.6.6 build). Fix: `repair_cpersona_rename_collision` runs before the sqlx migrations in `init_db`, merging legacy grants, dropping duplicate `plugin_configs`, and deleting the legacy `mcp_servers` row so the checksummed migration no-ops. The buggy migration file itself is intentionally unchanged (sqlx checksum). (#169)
- **bug-392** (MEDIUM): marketplace uninstall resolved the on-disk directory via the catalog cache and fell back to `server_id`; when a server had no catalog entry and its install dir differed from its id (e.g. `mind.deepseek` → `deepseek/`), `remove_dir_all` was silently skipped, orphaning the installed files. (#170)
- **bug-393**: the dashboard discarded kernel error response bodies, surfacing only a bare `statusText` (the long-standing "Bad Request" mystery on already-installed servers). It now surfaces the real kernel error message. (#171)
- **Tarball shared-prefix detection** — real GitHub archives open with a `pax_global_header` + top-level dir entry, so `detect_shared_prefix` returned `None`; `extract_tarball_stripped` then kept the `<repo>-<ref>/` wrapper and subdir selection matched nothing (a latent bug affecting every `raw_url` install to date). (#174)

### Added

- **bug-394 catalog seal verification** (HIGH): the catalog seal is `HMAC(hub master key, canonical message)`, but the kernel spawn-check computed `HMAC(local seal.key, file bytes)` — they can never match, hard-blocking every fresh-kernel install with "Magic Seal verification failed". Landed in two steps: an interim keyless entry-point integrity check + local re-seal (#176), then the proper Ed25519 catalog-seal verification via hub JWKS with a "cannot-verify vs verification-failed" two-class model and a 1 h in-memory JWKS cache (kid-miss refetch) (#177). `raw_url` tarball transport integrity remains separately enforced via sha256. (#175, #176, #177)

### Verified

- All ClotoCore-side fixes confirmed by `scripts/verify-issues.sh` (bug-388 repair-guard present; bug-389–392 patterns absent) and by fresh-kernel end-to-end installs through the hub-mediated distribution chain (catalog → proxy → blob → subdir extract → common → Ed25519 seal verified → Connected with 42 tools), including the D2 three-branch matrix (verified → Connected; JWKS-unavailable → untrusted; tampered → TAMPER SUSPECT hard-block).

---

## [0.6.8-alpha.3] — 2026-05-28

Verify-automation hardening. Third prerelease in the 0.6.8 line, published on the alpha channel. Pure infrastructure / tooling fix discovered during the first end-to-end run of `scripts/proxmox-windows-verify.sh` after `v0.6.8-alpha.2` published. No kernel / dashboard / installer behaviour changes.

### Fixed

- **`scripts/proxmox-windows-verify.sh`** — `capture_fingerprint()` was passing the PowerShell `FINGERPRINT_PS` heredoc through a doubly-nested `"…\"…\""` quote chain. Windows OpenSSH delivers remote commands through cmd.exe, which strips inner quotes the heredoc relied on for path strings like `"C:\Program Files\cloto-system"`. PowerShell then parsed `Test-Path C:\Program Files\cloto-system` as `Test-Path C:\Program` plus an unrecognized positional `Files\cloto-system`, aborting the fingerprint with a `PositionalParameterNotFound` error. Fix: switch literal paths to PowerShell single quotes, use `Join-Path` for `$env:APPDATA` derivations, and ship the entire payload via `powershell -EncodedCommand` (Base64 UTF-16LE) so cmd.exe sees no special characters at all.

### Verified

- End-to-end Sandbox verify for `v0.6.7 → v0.6.8-alpha.2` upgrade path on the Proxmox Windows guest:
  - **8/8 assertions PASS** (NO-OP hook path — Tauri default upgrade flow, no legacy migration needed).
  - **Data preservation confirmed**: dummy `cloto_memories.db` SHA-256 unchanged across upgrade.
  - **Wall clock: ~6 min 20 s** (rollback → guest SSH → download FROM → seed DB → silent install FROM → PRE fingerprint → download TO → silent install TO → POST fingerprint → assertion), well under the α→β promotion criterion of 20 min.
- Together with Pattern-C (landed in `0.6.8-alpha.2`) and the verify automation (`0.6.8-alpha.1`), this satisfies all three α→β promotion criteria. The 0.6.8 line is eligible for beta.1 promotion.

---

## [0.6.8-alpha.2] — 2026-05-28

Pattern-C capability tool name registry. Second prerelease in the 0.6.8 line; like alpha.1, deliberately published on the alpha channel so existing stable installs do not receive an in-app upgrade prompt. This release lands the structural refactor planned as PR #1 of that programme — the kernel no longer hard-codes MCP tool names as string literals in `handlers/system.rs`. It is the α→β promotion-criterion piece for the 0.6.8 line.

### Added

- **`ToolKind` enum** (`crates/core/src/managers/capability_dispatcher.rs`) — 11 well-known tool variants (`Store`, `Recall`, `ListMemories`, `ListEpisodes`, `ArchiveEpisode`, `UpdateProfile`, `Think`, `ThinkWithTools`, `AnalyzeImage`, `Transcribe`, `Speak`) plus `Custom(String)` escape hatch for non-well-known tools.
- **`CapabilityType::Display` / `FromStr`** — single source of truth for the JSON keys (`"Memory" | "Reasoning" | "Vision" | "Stt" | "Speech"`) used both by the new `build_from_capabilities` ingest path and by server-side `tools_for_capability` declarations.
- **`MgpServerCapabilities.tools_for_capability`** vendor extension (`Option<HashMap<String, Vec<String>>>`) — MCP servers can now explicitly declare which tools they expose under each capability, overriding the kernel's heuristic `classify_tool` fallback. Targeted for spec formalization in MGP 0.7.0 (Layered Manifest Layer 1/2, see the mgp-spec design).
- **`McpClientManager` ToolKind shims** (`call_kind`, `call_kind_at`, `call_kind_streaming`, `call_kind_streaming_at`, `has_kind`, `has_kind_at`) — typed entry points that keep handlers/ free of tool name string literals.
- **Tests** — 7 new unit tests in `capability_dispatcher.rs` (round-trip, capability mapping consistency, cross-capability rejection, Custom fallback, Display round-trip) and 3 serde tests in `mcp_mgp.rs` for the new field.

### Changed

- **`handlers/system.rs`** — 17 call sites swapped from string-literal tool names to `ToolKind` (categories: 11 direct `call_server_tool`, 1 `has_server_tool`, 3 `call_capability_tool` arg literal, 1 streaming, 1 event-dispatch string comparison). The `get_profile` tool, which is not in any well-known capability whitelist, uses `ToolKind::Custom("get_profile".to_string())` with an explicit `server_id` (escape-hatch convention).
- **MCP handshake flow** (`crates/core/src/managers/mcp.rs:1417`) — capability mappings are now built from the server's `tools_for_capability` manifest when present, falling back to the legacy `classify_tool` heuristic only for backward compatibility with servers that have not adopted the extension. **No behavior change for current servers** — none emit the field yet.

### Fixed

- **bug-289** (HIGH P2): `"think_with_tools"` literal no longer present in `crates/core/src/handlers/system.rs`. Tool name dispatched via `ToolKind::ThinkWithTools` through `call_kind_at` / `has_kind_at` / `call_kind_streaming_at` shims.
- **bug-290** (HIGH P2): `"archive_episode"` (and the rest of the Memory-capability literals — `store`, `recall`, `list_memories`, `list_episodes`, `update_profile`) no longer present. ARCHITECTURE.md §1.2 Capability-over-Concrete-Type compliance restored for these dispatch paths.

### Known limitations

- `classify_tool` private fallback list remains in `capability_dispatcher.rs` for servers that have not declared `tools_for_capability`. Removal is scheduled for 0.6.9+ once every shipped server has adopted the manifest path. Server-prefix heuristics (`memory.*`, `mind.*`, `vision.*`, `stt.*`, `output.*`) are also still present and tracked separately under §1.2 audit.
- bug-282 / bug-285 / bug-293 / bug-296 (other §1.2 violations from the 5-PR remediation plan) are unaffected by this release and remain open.

---

## [0.6.8-alpha.1] — 2026-05-27

Verify automation MVP release. First prerelease in the 0.6.8 line, deliberately published on the alpha channel so that the Tauri Updater (which resolves `latest.json` via `/releases/latest/download/` and therefore skips entries flagged `prerelease`) does NOT push it to existing stable installs. Production `v0.6.7` users will not see an in-app upgrade prompt; the alpha is reachable only via manual download from the Releases page. This structural isolation is exploited intentionally so verify automation regressions cannot realize on production user machines while the automation itself is being iterated.

### Added

- **NSIS hook structural gate** (`scripts/check-nsis-hook.sh` + new `nsis-hook-gate` CI job). Source-level grep assertion that the bug-386 `NSIS_HOOK_PREINSTALL` macro in `dashboard/src-tauri/installer.nsh` remains intact (macro entry point, legacy `Uninstall\cloto-system` registry probe, legacy silent-uninstall `ExecWait '"$0" /S'` invocation, and the `bug-386:` audit log line). Fails CI in <1 sec with a GitHub Actions `::error::` annotation if any required pattern is missing — silent removal of the hook now becomes impossible without an explicit gate update.

- **Proxmox Win11 verify driver** (`scripts/proxmox-windows-verify.sh`, ~250 LOC). Mac-side shell driver that rolls the Windows guest back to its pristine snapshot, downloads the FROM-version installer from GitHub Releases, seeds a dummy database to detect data preservation, runs silent install, captures a 9-field Windows fingerprint (install paths, both `HKLM` + `HKCU` uninstall keys, current `DisplayVersion`, db file SHA-256), then transports the TO-version installer (local file via `scp` or downloaded via `gh release`), runs silent install, captures the POST fingerprint, and diffs against an assertion matrix selected by the FROM version: hook-PRIMARY path for `0.6.5` (legacy productName migration) and hook-NO-OP path for `0.6.6+` (Tauri default in-place upgrade). Single iteration: ~6 min, vs ~30-45 min for Windows Sandbox manual verify.

- **NSIS-touching PR detector** (`.github/workflows/nsis-touching-detect.yml`). Triggered on `pull_request` open/synchronize/reopen, scans the diff for changes to `installer.nsh` (always flagged), `tauri.conf.json` (only when `bundle` / `productName` / `identifier` / `windows` keys touched), or any `Cargo.toml` `[[bin]]` block / `name = ` field. When a match is found and the PR title does not contain `[no-sandbox]`, applies the `nsis-touching` label (auto-creates on first use) and posts a comment instructing reviewers to run either `scripts/proxmox-windows-verify.sh` or a manual Sandbox verify pre-merge. Opt-out is recorded in the PR title for audit transparency.

### Note (alpha channel)

The 0.6.8 line operates under an α/β promotion pattern: `0.6.8-alpha.N` for feature / refactor iteration, `0.6.8-beta.N` for soak after feature-freeze, `0.6.8` stable for the cumulative release. The Tauri Updater's stable-only resolution (memory `clotocore-0.6.5-bug-385-release-land-30c6bd9-20260524` Known limitation) keeps every alpha and beta tag off the auto-update channel. This automation MVP lands first so subsequent alphas (the P1/P2 patches, the `config.rs:37 data_dir` literal migration, and other backlog work) can rely on it.

---

## [0.6.7] — 2026-05-26

CRITICAL hotfix release. Restores the auto-update path for v0.6.5 users (broken by the v0.6.6 `cloto-system` → `ClotoCore` product rename), unblocks marketplace installs of any server that declares env vars, and removes the silent 404 window during the first seconds after a release publish. Cumulative since v0.6.6.

### Fixed

- **bug-386 (CRITICAL — Auto-update)** — `Tauri Updater downloadAndInstall()` from v0.6.5 to v0.6.6 surfaced "Failed to apply update" because the v0.6.6 NSIS installer (with `productName = ClotoCore`) could not detect or migrate the v0.6.5 install registered under `productName = cloto-system`. New `dashboard/src-tauri/installer.nsh` injects a Tauri `NSIS_HOOK_PREINSTALL` macro (wired via `bundle.windows.nsis.installerHooks`) that probes `HKLM` + `HKCU` for the legacy `Software\Microsoft\Windows\CurrentVersion\Uninstall\cloto-system` key (covering `installMode: "both"`) and, when found, silently invokes the legacy uninstaller before the new install proceeds. The Tauri 2.x uninstaller's `un.SEC_APPDATA` section is not selected by default under `/S`, so user data at `%APPDATA%\Roaming\cloto-system\` survives the migration — `config.rs:37` still resolves `data_dir` to that path post-upgrade, preserving chat history, agent state, embedding namespaces, `mcp_access_control` grants, and registered MCP servers. The new install lands at `{autopf}\ClotoCore\` and registers a fresh uninstall key under the new product name. The hook is a no-op on fresh v0.6.7 installs (no legacy uninstall key present).

- **bug-387 (CRITICAL — Marketplace install)** (PR #152, master = `1d1be73`). `dashboard/src/components/mcp/InstallDialog.tsx` and the `EnvVarDef` TypeScript interface in `dashboard/src/types.ts` still read `.key` from env var definitions after `mgp-sdk` renamed the field to `.name` (with `#[serde(alias = "key")]` covering only the deserialize path). Every env var field collapsed into a single `{undefined: …}` state slot, every input rendered the same value, and the install request body posted `{env: {"undefined": <value>}}` which the kernel marketplace handler rejected with HTTP 400. Replaces seven `.key` references with `.name` and aligns the dashboard interface with the catalog wire shape; covered every marketplace server that declares env vars (CPersona, cmemo, cscheduler, etc.) on v0.6.6.

- **release.yml race** (PR #151, master = `caa94d9`). Switched the GitHub release workflow to a two-phase upload: binaries, signatures, and checksums upload first; `latest.json` (the Tauri Updater's "available version" indicator) uploads only after every other asset is in place. Closes the 404 window where `latest.json` declared v0.6.6 was available while `ClotoCore_0.6.6_x64-setup.nsis.zip` was still uploading.

### Backward compatibility

- v0.6.5 users upgrading via the in-app Tauri Updater path get the legacy install silently uninstalled and replaced with `C:\Program Files\ClotoCore\` — chat history and agent state preserved transparently because `%APPDATA%\Roaming\cloto-system\` is untouched.
- v0.6.6 users (rare — bug-386 blocked most of them at update time) continue to upgrade through the normal Tauri default flow since their registry uninstall key is already under `ClotoCore`; the new PREINSTALL hook simply no-ops.
- Fresh v0.6.7 installs are unaffected by the hook and land at `{autopf}\ClotoCore\` with a single clean uninstall key.

### CI / QA

- Windows Sandbox in-place upgrade verified pre-tag: v0.6.5 install → silent run of v0.6.7 NSIS → confirmed legacy uninstaller ran, `%APPDATA%` data preserved, fresh `ClotoCore` folder created, new uninstall registry key written, v0.6.5 chat history visible in the upgraded app.
- Marketplace install regression verified pre-tag: CPersona v2.4.21 dialog renders six env vars with distinct names and defaults, submit returns 200, `installed_servers` row created, kernel logs show no `git` invocation during `uv pip install` (vendored `mcp-common` path).

### Known limitations

- `config.rs:37 data_dir` literal remains `"cloto-system"` (preserved across the rename for data safety). A future release will migrate the literal to `"ClotoCore"` together with an explicit data-directory move step.

---

## [0.6.6] — 2026-05-24

Kernel structural cleanup release. Two long-pending architectural threads ship together in a single release with bisect-friendly per-PR isolation: (1) the surface product is renamed `cloto-system` → `ClotoCore` to align with the three-layer doctrine (`ClotoCore + ClotoCloud + Cloto app + ClotoHub`), and (2) the kernel is decoupled from hardcoded knowledge of any specific memory plugin id. Cumulative since v0.6.5.

### Changed

- **Product rename `cloto-system` → `ClotoCore`** (PR #136). `productName` in `tauri.conf.json`, the Cargo `[[bin]]` name (`cloto_system` → `clotocore`, separate from the existing `cloto` Magic Seal CLI), all user-facing display strings (`"Cloto System"` / `"CLOTO SYSTEM"` → `"ClotoCore"` / `"CLOTOCORE"`), `cli.rs` invocation help / version banner / self-update asset lookup pattern (`cloto_system-{target}` → `clotocore-{target}`), platform service `DisplayName` / systemd `Description`, dashboard tray tooltip, i18n titles, NSIS release artifact branding, install scripts (`install.ps1` / `install.sh`), and `docs/INSTALLER_DISTRIBUTION.md` artifact examples. Preserved invariants (rename-safe — would break in-place upgrade): `identifier = com.cloto.app`, `cli.rs default_prefix` (`/opt/cloto`, `C:\ProgramData\Cloto`), macOS launchd `SERVICE_LABEL = com.cloto.system`, Linux systemd `SERVICE_NAME = cloto`, Windows `sc.exe SERVICE_NAME = Cloto`, `config.rs:37 data_dir = ".../cloto-system"` (existing users' chat history / agent state / embedding namespaces live under this path).

- **Memory plugin decouple Phase A + B** (PR #137). `config.memory_plugin_id: String` → `Option<String>` (env empty/unset → `None`); `db::init_db(..., memory_plugin_id: Option<&str>)` signature change; `plugin_configs.database_url` INSERT gated on `Some(_)` so kernel boots successfully without an embedded-plugin seed. `handlers/marketplace.rs::register_server` gained a `bind_default_memory_if_unset` helper — when a `category == "memory"` plugin installs, the helper mirrors its id into `agent.cloto_default.metadata.preferred_memory` (only if absent / empty, never clobbers a user's manual choice). 41 test fixtures across `tests/` / `benches/` / `src/test_utils.rs` / `db/mod.rs` unit tests switched `init_db(..., "memory.cpersona")` → `init_db(..., None)`. Aligns with MGP §10 invariant 3 (open standard, no privileged plugin).

- **Memory plugin id rename `memory.cpersona` → `cpersona`** (PR #138). Catalog-canonical id propagated across dashboard preset lists (`presets.ts` MINIMAL / STANDARD / ADVANCED / EXPERT_SERVERS), `SetupWizard.tsx` `ALL_SELECTABLE_SERVER_IDS`, i18n keys (`server_memory_cpersona` → `server_cpersona`), `handlers_http_test.rs` test fixtures, and `README.md` MCP server table. Intentionally preserved: historical migration SQL literals (sqlx checksum constraint), `marketplace.rs::effective_install_dir` legacy backward-compat tests, `handlers/mcp.rs` dotted-id acceptance test, `capability_dispatcher` test fixtures, `format.test.ts` namespace-stripping assertion, and 5 design docs that discuss both old/new ids (deferred to follow-up editorial pass).

### Migrations

- `20260524000000_backfill_default_memory.sql` — back-fills `agent.cloto_default.metadata.preferred_memory = 'cpersona'` for existing users whose pre-0.6.6 memory binding came from the env default (`CLOTO_MEMORY_PLUGIN_ID`) rather than dashboard selection. Idempotent: no-op when `preferred_memory` already has a value or when no memory plugin is installed yet.
- `20260524010000_rename_memory_cpersona_to_cpersona.sql` — renames `mcp_servers.name` `memory.cpersona` → `cpersona`, re-issues the `mcp_access_control` `server_grant` row under the new id (DELETE + INSERT around the parent rename — mirrors the KS22 rename precedent at `20260309000000`, FK requires this shape), renames `plugin_configs.plugin_id`, and cosmetically normalises any `agent.metadata.preferred_memory = 'memory.cpersona'` row to `'cpersona'`. Idempotent: WHERE clauses target only legacy rows.

### Backward compatibility

- Users with `CLOTO_MEMORY_PLUGIN_ID=memory.cpersona` in env keep their pre-0.6.6 behavior (env value flows through as `Some(...)`, plugin_configs row written as before). Phase D migration then renames the row so `cpersona` becomes the canonical id.
- `init_db(..., None)` is a new valid state — the kernel boots with no memory plugin pre-seeded, then the modern Setup Wizard / marketplace install path populates `preferred_memory` on the first memory-kind install (via `bind_default_memory_if_unset`).
- Existing chat history, agent state, embedding namespaces, mcp_access_control grants — all preserved (`config.rs:37 data_dir` and `tauri.conf.json identifier` are explicitly held constant).

### CI / QA

- Test count: 324 tests pass (322 pre-0.6.6 + 2 new migration tests in `migration_test.rs` for Phase C+D smoke and Phase C idempotency).
- `cargo fmt --all -- --check` + `cargo clippy --workspace --exclude app -- -D warnings -A …` (CI lint suppressions) + `npx biome lint src/` all clean.

### Fixed-points (irreversible from this release)

- `config.memory_plugin_id: Option<String>` (downstream test fixtures depend on Option semantics)
- `agent.metadata.preferred_memory` is the single source of truth for per-agent memory plugin selection
- `init_db` signature `Option<&str>`
- Cargo `[[bin]] name = "clotocore"` (cascades to CI artifact names + install scripts + platform path constants)
- `mcp_servers.name = 'cpersona'` for the CPersona plugin (no `memory.` prefix); legacy rows migrated and gone after first boot on 0.6.6+

---

## [0.6.5] — 2026-05-24

CRITICAL hotfix release. Restores the auto-update path for all installer-based ClotoCore installs by switching the dashboard from a broken sidecar shell-out to the configured Tauri Updater plugin. Cumulative since v0.6.4 (2026-05-23).

### Fixed
- **bug-385 (CRITICAL — Auto-update)** — `applyUpdate()` in `dashboard/src/lib/tauri.ts` shelled out to a sidecar named `cloto_system` via `@tauri-apps/plugin-shell`, but the binary was never bundled into the NSIS / DMG / `.deb` installer (`bundle.externalBin` is unset in `tauri.conf.json` and the release workflow never copies the standalone kernel binary into the app bundle). Every "Update Now" click failed at `Command.create()` with a program-not-found error which the UI surfaced as the default "Failed to apply update" message — affecting all installer-based ClotoCore installs since v0.6.3. The Tauri Updater plugin itself was already fully configured (pubkey + endpoint pointing at `latest.json` + `createUpdaterArtifacts: "v1Compatible"`), and the release workflow generates a valid `latest.json` per release — the dashboard simply never invoked it. `checkForUpdates()` now uses `@tauri-apps/plugin-updater` `check()` (replacing direct GitHub API calls and freeing the path from the 60-request/hour unauthenticated rate limit); `applyUpdate()` calls `check()` + `update.downloadAndInstall()` + `@tauri-apps/plugin-process` `relaunch()` (with `try/catch` around `relaunch()` for NSIS install paths where the new installer kills the running process). The now-dead `shell:allow-execute cloto_system` entry was removed from `capabilities/default.json`. Existing v0.6.3 / v0.6.4 users must download v0.6.5 manually one last time to recover the in-app auto-update flow.

### Known limitations
- `latest.json` resolves through GitHub's `/releases/latest/download/` URL which only points to stable releases. Pre-release channel discovery (previously approximated by reading 30 release entries via the GitHub API) is deferred to a future feature — pre-release users on v0.6.X-beta.Y will not receive in-app upgrade prompts for v0.6.X-beta.Z.

---

## [0.6.3] — 2026-05-20

First stable release of the 0.6.3 line. Promotes `0.6.3-beta.14` after coordinated Discord T1 session-continuity work and CRITICAL Setup Wizard fixes. Cumulative since v0.6.3-beta.13.

### Added
- **T1 SessionManager (kernel-owned short-term transcript)** — new `SessionManager` keeps an in-memory `AgentSession` per `(agent_id, bridge_session_id)` with bounded transcript + `tool_history`. Replaces stateless context-injection between Discord bridge and kernel: Discord callbacks no longer refetch channel history via REST or traverse 3-hop reply chains, and the agentic loop consumes T1 as the authority source for short-term context. CPersona handles long-term memory only.
- **Magic Seal verification** — `cloto seal generate/verify` CLI binary; `mgp-seal` crate integration (cut from external repo to `mgp-rs` workspace); `RegistryEntry.seal` threaded through DB + runtime config; verified/unverified badge on `MarketplaceCard`; force-untrusted on missing seal per MGP v0.6.3 §10 invariant 3.
- **Docker source dispatch** — `install_from_docker` for `SourceSpec::Docker` (bare-command whitelist + `docker pull` + `-e KEY` no-value secret-leak guard + `BTreeSet` env dedup). Companion to the new run_install dispatch on `install.source.kind` (Git / RawUrl / Pypi / Docker).
- **Marketplace catalog flip** — default catalog URL switched to `https://hub.cloto.dev/api/catalog` (Phase 5d-2). The legacy `raw.githubusercontent.com/Cloto-dev/cloto-mcp-servers` tarball template is retained for monorepo install path only; new catalog entries dispatch via the typed `SourceSpec` runner.
- **Marketplace install helper** — `effective_install_dir` helper extracted; uninstall fallback aligned; empty `install.directory` entries now probe the correct directory.

### Fixed
- **bug-359/360/361/362 (CRITICAL — Setup Wizard)**:
  - `installingRef.current` re-entrancy guard closes the prior `EventSource` before opening a new one on retry (bug-359 — multiple concurrent SSE connections).
  - `applyingRef.current` re-entrancy guard suppresses double-click duplicates on step 4 preset apply (bug-360).
  - Back-from-step-5 navigation resets installation state via `setInstallStarted(false)` + EventSource close + ref clear so step 4 re-entry is clean (bug-361).
  - `setup-complete.json` is now written atomically via `setup-complete.json.tmp` + rename, eliminating the corrupted-file → re-setup loop (bug-362).
- **bug-287 (CRITICAL — Access Control)** — removed the hardcoded `KERNEL_NATIVE_TOOLS` allowlist from `registry.rs`; renamed `create_mcp_server` to `mgp.kernel.create_mcp_server` so it is dispatched by the `mgp.` prefix like every other kernel-native tool. Access control still routes through `resolve_tool_access(pool, agent_id, "kernel", tool_name)`; `mcp_servers` has no `name='kernel'` row so the default policy is opt-in → Deny without an explicit grant. Tool name renamed across `mcp_kernel_tool.rs` (schema + `TOOL_NAME_CREATE_MCP_SERVER` const), `mcp.rs` (dispatch match), `system.rs` (rejection compose test), `tool_rejection_smoke.rs`, `MGP_SPEC.md`, `MCP_PLUGIN_ARCHITECTURE.md`.
- **bug-286 (HIGH — Memory Recall Contract)** — kernel now owns short-term context via SessionManager (T1) and asks the memory plugin only for long-term `recall`. The local payload variable was renamed and the call switched from `recall_with_context` to `recall`, so the original `recall_args` symbol no longer appears in `handlers/system.rs`. Full constant-ification of memory tool names is deferred to 0.6.4 (bug-290).
- **bug-310 (HIGH — Recall Timestamp Sort)** — `parse_mcp_recall_result` now sorts the parsed messages by timestamp ascending (`result.sort_by_key(|m| m.timestamp);`) before returning, guaranteeing chronological order for engines that expect oldest-first.
- **bug-344 (HIGH — Cross-User Memory Contamination)** — CPersona `recall` now accepts a `source_id` prefix filter (cpersona v2.4.20); the kernel derives `recall_source_id` from `msg.source` (User → `id`, otherwise empty for v2.4.19 fallback) and threads it into the MCP recall payload at `handlers/system.rs:504`. The empty fallback preserves backwards-compatible all-users recall for Agent / System messages.

### Changed
- **Marketplace install dispatch** — `RegistryEntry` / `EnvVarDef` cut over to `mgp-sdk` v0.2.0; `run_install` now dispatches on `install.source` (Git / RawUrl / Pypi / Docker) via the typed `SourceSpec` runner instead of the legacy monorepo path. Legacy monorepo install retained for back-compat.
- **mcp-seal module → mgp-seal crate** — internal `mcp_seal` module swapped for the `mgp-seal` crate (now sourced from `mgp-rs` workspace, no external repo dependency).
- **Phase 5 cutover prep** — URL helpers extracted; catalog fetch now exposes a `Stale { cached, error }` variant so the UI keeps showing the prior catalog when the origin is briefly unreachable.
- **Documentation language** — `MGP_SPEC.md` retargeted as a pointer to the `mgp-spec` repo; small-touch Japanese strings translated to English across 5 files; `MCP_STARTUP_PERFORMANCE` analysis translated to English; `V0_3_CHAT_UX` archive entry exempted + stale archive entries pruned.
- **Git hooks** — `.githooks/pre-commit` and `scripts/install-hooks.sh` are now source-controlled (was per-clone manual install).
- **Dependencies** — `tauri-plugin-single-instance` 2.4.0→2.4.2; `tauri-plugin-dialog` 2.7.0→2.7.1; `tracing-appender` 0.2.4→0.2.5; `tauri-build` 2.5.6→2.6.1; `jsdom` 29.0.1→29.1.1 (dev); `sigstore/cosign-installer` 4.1.1→4.1.2 (CI).

### Security
- **Force-untrusted on missing seal** (MGP v0.6.3 §10 invariant 3) — connectors without a valid Magic Seal are forced to `untrusted` trust level on install, independent of the server's self-declared trust level. Closes the curation-layer bypass where a malicious connector could self-declare `core` trust.

### CI / QA
- `qa/issue-registry.json` updated: 8 bugs marked fixed in 0.6.3 (bug-286/287/310/344/359/360/361/362) with `fix_note` referencing the responsible phase and commit. `scripts/verify-issues.sh` PASS (87 verified / 134 fixed / 0 stale / 0 errors). Fix marker patterns intentionally surface as `[VERIFIED]` (= fix marker present) rather than `[FIXED]` (= bug pattern absent) for the 6 in-place modifications; the script's PASS/FAIL gate is unaffected.

### Companion releases
- `cpersona` v2.4.20 — `source_id` prefix filter, fixes bug-344 cross-user memory contamination at the memory-plugin layer.
- `cloto-mgp-discord` v0.5.0 — deletes per-callback Discord REST history fetch and 3-hop reply chain traversal (183 LOC net reduction); relies on the kernel's T1 transcript instead.

---

## [0.6.3-beta.6] — 2026-04-05

### Added
- **Health Check: venv detection and repair** — scan now checks Python venv existence and version mismatch; repair rebuilds venv and reinstalls all server dependencies
- Uninstall data cleanup documentation in README

### Changed
- **Batch install optimization** — replaced per-server sequential `pip install` (N × 120s) with single unified `pip install` for all Python servers, matching the pattern from `mcp_venv.rs`
- **SetupWizard title bar** — now uses shared `ViewHeader` component instead of custom header

### Fixed
- TypeScript type check failure in `repairHealth` API call (`authFetch` → `mutate`)

---

## [0.6.3-beta.5] — 2026-04-05

### Added
- **Kernel Health Check System** — self-diagnostic scan with auto-repair for database integrity issues
  - Quick Scan: 5 checks (DB connection, orphaned chat messages, orphaned trusted commands, orphaned permission requests, audit chain integrity)
  - Standard Repair: automatic cleanup of orphaned records
  - Startup scan: runs automatically on boot (configurable via `CLOTO_HEALTH_SCAN_ON_STARTUP`, default: on)
  - API: `GET /api/health/scan`, `POST /api/health/repair`
- **Settings > Health tab** — dashboard UI for scan results, manual scan/repair buttons
- Japanese translation for Health tab

---

## [0.6.3-beta.4] — 2026-04-05

### Security
- **Tar path traversal prevention** — marketplace install now validates extracted paths stay within target directory (zip-slip mitigation)
- **Missing authentication** — `get_agent_access` endpoint now requires API key (was unauthenticated unlike all other endpoints)
- **Code validator bypass** — blocked pattern matching now uses case-insensitive comparison, preventing `Eval()`/`EXEC()` bypass
- **Revoked key check logging** — lock acquisition failure during revoked key check is now logged instead of silently skipped

### Fixed
- **UTF-8 panic** — tool hint truncation now uses char-based indexing instead of byte slicing, preventing panic on multi-byte characters
- **Negative chat limit** — `limit` parameter now clamped to minimum 1, preventing `usize` wrapping on negative input
- **Iteration overflow** — agentic loop counter uses `saturating_add` to prevent theoretical u8 overflow
- **Script path inconsistency** — dynamic MCP server script restoration now uses same path (`data/mcp_scripts/`) as creation
- **Attachment storage path** — uses `state.data_dir` instead of relative path, consistent with VRM/avatar storage
- **Non-transactional agent deletion** — `delete_agent` now wraps all DB operations in a transaction for consistency
- **DB timeout gaps** — added `db_timeout` to all cron (8 functions) and LLM (5 functions) DB operations
- **Audit log timeout** — `write_audit_log` transaction now wrapped with configurable timeout
- **Mutex poison recovery** — `StreamAssembler` and `ToolIndex`/`SessionToolCache` now recover from poisoned mutexes instead of panicking

### Added
- `MgpCapabilities` helper usage documented in quickstart guide
- Coming Soon placeholders for non-functional log UI elements

---

## [0.6.3-beta.3] — 2026-04-04

### Fixed
- **MCP server toggle race condition** (Issue #65) — `stop_server()` now waits for child process exit before returning, preventing DB lock conflicts on restart
- **Safe integer casts** — `i64→i32` (cron), `usize→u8` (delegation chain), `u64→u8` (cron generation) now use `try_from` instead of `as` casts
- **Error logging** — Cargo.toml read errors in marketplace and RwLock poison recovery now logged instead of silenced
- **CVE-2026-33672** — picomatch 2.3.1→2.3.2, 4.0.3→4.0.4 (glob matching method injection)

### Added
- [MCP/MGP Server Quickstart](QUICKSTART_MCP_SERVER.md) — two-path guide for new server developers
- `CLOTO_YOLO_EXCEPTIONS` documented in README configuration table
- Tauri dev note in Quick Start section

### Changed
- README: test badge 351→234, security section +3 items, documentation links updated
- CHANGELOG: removed internal marketing references from beta.1 and beta.2 entries
- CLAUDE.md: clarified issue registry as anti-hallucination tool

### Dependencies
- jsdom 28.1.0→29.0.1 (dev)

---

## [0.6.3-beta.2] — 2026-04-04

### Security Hardening (8 layers)
- **L2**: Kernel tool RBAC — `mgp.*`/`gui.*` tools now checked via `resolve_tool_access(server_id="kernel")`; default Allow, explicit Deny entries restrict specific agents
- **L3**: YOLO mode exceptions — `CLOTO_YOLO_EXCEPTIONS` env var (default: `filesystem.write,network.outbound`); excepted permissions require approval even in YOLO mode
- **L5**: Merkle chain audit log — `chain_hash` column on `audit_logs` table; each entry hashed with SHA-256(previous_hash | canonical_data) for tamper detection
- **L8**: Runtime host whitelist audit — `add_host()` logs a warning when a new host is added
- **L9**: Event depth hardening — `MAX_EVENT_DEPTH` cap lowered from 50 to 25; warning logged at depth > 5
- **L11**: MGP permission declarations — avatar and discord servers declare `permissions_required: ["network.outbound"]` in initialize response (cloto-mcp-servers)
- **L12**: destructiveHint HITL gate — parse MCP `annotations.destructiveHint`, require approval for destructive tools via existing command approval flow
- **L14**: Trust level mismatch warning — kernel logs when server self-declares higher trust than config allows

### Added
- `McpTool.annotations` field for MCP tool annotation parsing
- `McpClientManager::is_tool_destructive()` helper
- `CLOTO_YOLO_EXCEPTIONS` environment variable
- Planned breaking changes section in PROJECT_VISION (§10)
- DB migration: `audit_logs.chain_hash` column

### Changed
- YOLO permission flow refactored to partition-based logic (auto-approvable vs excepted)
- `write_audit_log()` now uses single SQLite transaction for chain hash consistency

### Fixed
- 3 stale issue-registry bugs marked as fixed (bug-311, bug-314, bug-343)

### Documentation
- Security layer audit report (15 layers verified against code)
- GitHub Sponsors FUNDING.yml

---

## [0.6.3-beta.1] — 2026-04-03

### Added
- **Streamable HTTP** transport for remote MCP server connections
- Agentic loop for `ask_agent` tool execution chains
- Discord conversation context injection into LLM calls
- Discord callback metadata forwarding to agent messages
- CPersona **memory channel** support for channel-based context separation
- Actions panel with inter-agent dialogue visibility
- CRON job execution display in Actions Dialogues
- Engine selector on CRON job creation form
- Memory **export/import** UI in MemoryCore
- Marketplace changelog display on update-available cards
- MGP badge and glow effect on MCP server cards
- Agent processing glow indicator in sidebar
- Speaker name display on memory cards
- **IO category** for bidirectional MCP servers in dashboard
- Process relaunch on error boundary restart
- Marketplace actions locked in dev mode by default
- Pre-compute archive/profile via CFR engine in background
- Generalized `tool_hint` for direct tool execution bypass
- `CapabilityType::Speech` with capability-based **auto-speak**
- Installer (Experimental) section in README with setup wizard fix

### Changed
- `io.discord.karin` renamed to `io.discord`
- Per-agent Discord server entry template
- Dialogues tab bar replaced with **vertical scroll list**
- Agent description limit increased from 1000 to 5000 bytes

### Fixed
- MCP venv: parallel pip install replaced with **single invocation**
- Stale Python venv detection and automatic recreation
- pip install timeout and `--no-input` flags
- Python venv and cargo build timeouts
- Duplicate tool names in LLM tool schemas
- Null reference in Discord callback metadata
- Avatar **cache-bust** on re-upload
- Avatar vision analysis skipped when agent lacks Vision access
- CRON dialogue response pairing
- Memory card text size and description textarea height
- Export/import icon semantics corrected
- Speech tool schema exclusion limited to "speak" tool only
- Setup wizard download URL fixed (points to cloto-mcp-servers releases)
- Setup wizard server/venv paths corrected for production layout
- `detect_project_root()` recognizes `cloto-mcp-servers/` directory

### Security
- Authentication, cryptography, MCP server creation, and Tauri capabilities **hardened**
- GitHub Actions pinned to commit SHAs
- CVE-2026-33055, CVE-2026-33056 (tar crate update)
- Access control magic strings replaced with typed enums
- `aria-label` added to all interactive dashboard components

### Documentation
- Code quality audit report (65 findings — 2 critical, 19 high, all fixed)
- Documentation-codebase **integrity audit** (3 critical, 7 high, 5 medium fixed)
- CPersona design document updated to v2.4.6 (tool count, version table, architecture diagram)
- MGP specification kernel tool count corrected (17→25)
- GUI component map updated
- Test count corrected: 351 (234 Rust + 117 Python)

---

## [0.6.3-alpha.11] — 2026-03-21

### Changed
- MGP renamed from "Model General Protocol" to "**Multi-Agent Gateway Protocol**"
- Release assets consolidated from 34 to 22 (SHA256SUMS.txt replaces per-file checksums)

### Fixed
- Kernel startup failure no longer **silently ignored** — `start_kernel()` refactor with Tauri error dialog
- LLM proxy bind failure reported in background — no longer blocks HTTP server startup
- MCP deferred boot **race condition** resolved — `Arc<Notify>` replaces `yield_now()`
- Tauri tray icon panic prevented
- EventManager **mutex poisoning** cascade eliminated across 13 sites
- McpAccessControlTab infinite re-render loop
- Cross-platform MCP path normalization
- MCP config parse error visibility improved
- Faster **graceful shutdown** — concurrent drain with 10-second cap
- pip install timeout and `--no-input` to prevent setup hangs
- Stale venv auto-detection — compares Python major.minor, auto-recreates on mismatch
- Text size and color rule violations in dashboard

### Security
- `aws-lc-sys` updated (RUSTSEC-2026-0044, RUSTSEC-2026-0048)
- `rustls-webpki` updated (RUSTSEC-2026-0049)

---

## [0.6.3-alpha.10] — 2026-03-20

### Fixed
- **Empty MCP server list** after NSIS installation — `mcp.toml` now embedded in binary via `include_str!` and extracted to `data/mcp.toml` on first launch with snapshot pattern
- Removed broken Tauri `resources` bundling (Tauri v2 transforms `../` into literal `_up_` directories)

---

## [0.6.3-alpha.9] — 2026-03-20

### Fixed
- **Empty MCP server list** after installation — `mcp.toml` bundled as Tauri resource for first-launch discovery
- `exe_dir/mcp.toml` added as production fallback path
- `CLOTO_MCP_SERVERS` fallback probes multiple candidate directories (bundled, sibling repo, legacy layout)
- Always-true assertion removed from security forging test

---

## [0.6.3-alpha.8] — 2026-03-19

### Changed
- `.env.example` updated with Ollama config
- Outdated `CODE_QUALITY_REPORT.md` removed

### Fixed
- Dashboard: gate **console statements** behind `import.meta.env.DEV`
- Dashboard: improve catch block type safety, extract magic numbers, remove dead CSS
- Rename legacy `karin` color to `cloto`
- All **clippy warnings** resolved
- Warn logging added to silent I/O errors in system handler
- Benchmark helpers updated to match current `AppState` struct
- `ask_agent` tool description improved
- CI: cargo fmt violations and missing assertion fixed

---

## [0.6.3-alpha.7] — 2026-03-17

### Added
- **Rust MCP server** support in marketplace — servers with `runtime: "rust"` built with `cargo build --release`, with toolchain detection and build progress streaming
- Startup timing log (`startup: X.Xs`)
- Rust badge on marketplace cards
- MCP startup performance analysis report

### Changed
- MCP server connections **parallelized** — `connect_server_configs()` uses `join_all` for concurrent connections
- Parallel **venv dependency sync** — pip install runs concurrently for all servers
- Background venv sync — `ensure_mcp_venv()` moved off critical startup path
- **Startup time reduced from ~40s to ~7s**
- `output.avatar` removed from `mcp.toml` (marketplace-only distribution)

### Fixed
- Sidebar "Agents" nav not returning to agent selection
- Agent config screen highlighting wrong **sidebar** item
- Sidebar agent click not working after returning from chat/config
- Config screen persisting when navigating away
- Marketplace install blocked for config-loaded servers
- Cargo build failing in data dir due to parent **workspace detection**
- CI: cargo fmt, flaky seal key test, issue registry

---

## [0.6.3-alpha.6] — 2026-03-16

### Changed
- `mcp.toml` **portability**: `${CLOTO_MCP_SERVERS}` env var with sibling-repo fallback
- `resolve_servers_dir_from_config()` resolves relative paths against project root

### Fixed
- Update checker detects **pre-release** versions via `/releases` API
- Pre-release segment version comparison (alpha.4 vs alpha.5)
- Setup wizard Python pre-check with download link and **retry button**
- Hardcoded developer-machine paths removed from history

---

## [0.6.3-alpha.5] — 2026-03-16

### Added
- **VRM thumbnail extraction** and avatar offer dialog
- i18n: Japanese translations for VRM dialog and settings sections
- CFR default enabled for new routing rules
- Engine selection **persistence** per agent in localStorage

### Changed
- Deferred save pattern unified for all agent config
- `output.avatar` migrated to `cloto-mcp-servers` repository

### Fixed
- VRM upload metadata **COALESCE** race condition
- Null injection prevention in metadata
- Marketplace refresh bypasses server cache
- Duplicate refresh buttons unified

---

## [0.6.3-alpha.4] — 2026-03-16

### Added
- **Agent state persistence** across navigation (thinking steps, chat, SSE)
- Code block header bar with language label, copy, and download
- Artifact panel collapse/expand with **sessionStorage** persistence
- MCP server lifecycle feedback (spinner + checkmark)

### Fixed
- CI: MSI target exclusion, clippy errors, biome lint, sentinel assertion
- Workspace-wide **cargo fmt**

---

## [0.6.3-alpha.3] — 2026-03-16

### Added
- **Auto-update** check on startup (Tauri only, configurable)
- Discord-style update indicator in header
- Tool hint display shows actual command name

### Changed
- Unified **card styles** across all dashboard components

### Fixed
- Avatar upload/deletion **race condition**
- Avatar upload spinner hang with 30-second timeout
- Semver comparison for pre-release versions

### Documentation
- CPersona v2.5/v3.0 roadmap added

---

## [0.6.3-alpha.2] — 2026-03-16

### Added
- **MCP Server Marketplace** (Phase 1 & 2) — catalog, install, batch install with SSE progress
- **Setup flow unification** with SSE progress streaming
- Tier-1 rate limiting and startup dependency sync
- Biome formatter, lefthook pre-commit hooks, sentinel script
- 14 unit tests for marketplace and setup

### Fixed
- `resolve_servers_dir_from_config` **TOML parsing** failure (critical)
- Avatar deletion and AgentConsole TDZ crash
- Install path resolution fixes

---

## [0.6.3-alpha.1] — 2026-03-13

### Added
- **CPersona background task queue** (Phase 5) ported from predecessor
- SSE **SequencedEvent** with `Last-Event-ID` replay for reliable event streaming
- Cron `source_type` field to distinguish user vs system messages
- VRM thinking pose auto-application on agent thinking state
- `AgentThinking` event emission before all LLM calls
- Host OS info injected into agent system prompt

### Fixed
- **WebView2** startup crash (`ERR_CONNECTION_REFUSED`) on release builds
- `useLongPress` stale closure in long-press handler
- Chat message deletion not including system rows
- Web search false-positive health check

### Documentation
- VOICEVOX credit added to README
- CPersona **2.x versioning** adopted (inheriting KS2.x lineage)
- 8 internal audit docs moved to `.dev-notes/`
- Comprehensive `CPERSONA_MEMORY_DESIGN.md` update (20 fixes)

---

## [0.6.2] — 2026-03-11

### Added
- **VRM Avatar System** (Layer 1) — full procedural animation pipeline (breathing, blinking, micro-sway, gaze drift, agent state transitions, default pose with smooth transitions)
- **VRMA pose system** with direct quaternion application, smooth slerp-based transitions, drag-and-drop loading
- VRM **expression mapper** — cross-avatar compatibility layer with fallback chains
- **mgp-avatar MCP server** — VOICEVOX TTS with automatic viseme extraction for real-time lip sync
- MGP `set_pose` tool for avatar pose control (relaxed, attentive, thinking, arms_crossed)
- Auto-speak final LLM response with configurable bypass
- VRMA thinking pose preset (Blender-authored)
- Eye narrowing during thinking state
- Middle-click orbit rotation in VRM viewer
- Extended **bone controls** (neck, spine, head, hand)
- Core architecture refactor (Phase 1–8): **CapabilityDispatcher**, metadata JSON migration, unified state consolidation, process lifecycle safety, permission events, config-driven capabilities, agent type filtering
- **OS-level isolation** (MGP §8-10) for MCP server sandboxing
- MGP tool discovery **latency tier** scoring (Tier C/B/A/S)

### Fixed
- VOICEVOX `accent_phrases` field name for viseme extraction
- Pre-phoneme compensation for accurate **lip sync** timing
- Audio cutoff prevention and viseme desynchronization
- Bypass agentic loop for TTS (prevent prompt readback)
- VOICEVOX pipeline and sandbox path resolution

---

## [0.6.1] — 2026-03-09

### Added
- `ask_agent` kernel tool for inter-agent delegation
- `AgentThinking` SSE events for LLM intermediate reasoning display
- `gui.map` and `gui.read` kernel tools for dynamic UI documentation
- Agent config presets with deferred avatar save
- MGP dead code integrated into runtime (Phase 0-5)
- GitHub Issue Sync workflow: auto-create/close GitHub Issues from `qa/issue-registry.json`

### Changed
- Rename `memory.ks22` to `memory.cpersona` across entire codebase
- Auto-grant MCP access on agent creation
- Settings screen text sizes increased for readability

### Fixed
- Comprehensive data cleanup on agent deletion (bug-231)
- Dashboard API key delivery in Tauri mode and FK violations in MCP access control
- `useAgents` cache race condition, wizard UX improvements, and 6 pre-existing TS errors
- Dashboard UI/UX improvements and LLM thinking event support

### Documentation
- Backfill CHANGELOG entries for v0.6.0-alpha.4 through v0.6.0 stable (MGP content)

---

## [0.6.0] — 2026-03-08

### Added
- **MGP (Multi-Agent Gateway Protocol) Tier 1-4 implementation complete**
  - Tier 1: Security primitives — protocol-level access control and audit trails
  - Tier 2: Observability — monitoring, metrics, and diagnostic capabilities
  - Tier 3: Bidirectional communication — server→kernel notifications and tool discovery
  - Tier 4: Intelligence Layer — context management, adaptive behavior, and compliance
- 17 MGP kernel tools in `mgp.*` namespace (access control, audit, lifecycle, streaming, discovery)
- MGP server creation with coordinator pattern
- Priority boot sequence for MGP servers
- Tool discovery stress tests and context reduction measurements

### Fixed
- MGP Tier 1-3 spec compliance (bug-182 to bug-222)
- Missing Tier 4 tool schemas registered; `tool_history` sanitization hardened
- MGP kernel tool execution and LLM provider integration
- Stale connection status threshold removed for immediate disconnect detection
- Linux Tauri build deps: added libgbm-dev, libegl-dev, libxcb1-dev
- macOS CI: upgrade xcap 0.0.13 → 0.8
- Linux CI: switch to ubuntu-24.04 for libspa 0.9.2 compatibility

### Documentation
- MGP implementation roadmap added
- MGP documentation updated to reflect Tier 1-4 completion

---

## [0.6.0-beta.3] — 2026-03-07

### Added
- First-run setup wizard
- Agent config export/import

### Fixed
- Hide export button for default agent (Cloto Assistant)

---

## [0.6.0-beta.2] — 2026-03-07

### Added
- Modular i18n with react-i18next (EN + JA)
- Filesystem-based language packs with extended translations and text readability enforcement

### Removed
- Container agent type from dashboard

---

## [0.6.0-beta.1] — 2026-03-07

### Added
- Semantic cache for research server
- TTL-based LRU cache for query embeddings in KS22

### Removed
- Predecessor project references from codebase

---

## [0.6.0-alpha.5] — 2026-03-07

### Changed
- Codebase reduced by ~1,400 LOC with structural improvements

### Fixed
- 5 LOW bugs resolved, Python MCP test base added, 2 reclassified as wontfix

### Removed
- Orphaned `runtime_plugins` and `agent_plugins` tables dropped

---

## [0.6.0-alpha.4] — 2026-03-06

### Added
- Cross-platform Tauri desktop app support (Linux + macOS)
- macOS code signing and notarization configuration
- Configurable settings extracted from hardcoded values

### Fixed
- Version prerelease label auto-generation from package version

---

## [0.6.0-alpha.3] — 2026-03-06

### Fixed
- 8 MEDIUM bugs resolved, improved Python MCP server quality
- Graceful shutdown now broadcasts to all tasks (not just one listener)
- MCP stderr log noise suppressed

---

## [0.6.0-alpha.2] — 2026-03-05

### Removed
- CLI crate (`cloto_system` binary removed)
- Status UI page

### Fixed
- MCP server restore bug on kernel restart

### Security
- Authentication added to read-only APIs (agents, plugins, metrics, memories)
- YOLO mode audit log: all auto-approved actions recorded
- Revoked API keys now expire with TTL cleanup

---

## [0.6.0-alpha.1] — 2026-03-05

### Added
- MGP specification v0.6.0-draft: structural audit, architectural revision, split into maintainable part files
- SearXNG self-hosted search via Docker Compose
- Multi-provider search fallback chain for MCP
- Reliable chat message persistence with retry logic

### Changed
- Replace Inno Setup installer with Tauri NSIS installer (Windows)
- Dashboard: extract shared UI components and utility hooks

### Fixed
- MGP integrity scan findings resolved (S1-S3, I1-I3, X1)
- Windows console windows appearing from MCP server child processes
- Kernel images blocked by CSP `img-src` directive
- Release pipeline: Ed25519 signing, artifact paths, macOS runner, cosign verification

---

## [0.5.11] — 2026-03-04

### Changed
- Unified REST API response envelope (`{ "data": ... }` / `{ "error": ... }`)
- Auto-generate Tauri API key on first launch

---

## [0.5.10] — 2026-03-04

### Added
- Multi-user identity propagation across the full pipeline (chat, agentic loop, MCP tools, memory)

---

## [0.5.9] — 2026-03-04

### Fixed
- Memory contamination causing time hallucination in agent responses

---

## [0.5.8] — 2026-03-04

### Changed
- Dashboard UI/UX refinements: retry fix, MemoryCore design unification, engine selector polish

---

## [0.5.7] — 2026-03-04

### Added
- CRON autonomy security: recursion depth control and audit log guarantee

---

## [0.5.5] — 2026-03-04

### Added
- Gemini-style engine switcher in chat input bar

---

## [0.5.4] — 2026-03-04

### Added
- `tool.cron` MCP server: stateless CRON job management via kernel REST API (create, list, delete, toggle, run now)
- `tool.agent_utils` MCP server: 8 deterministic utility tools (time, math, date arithmetic, random, UUID, unit conversion, encode/decode, hash)
- Default MCP server grants for Cloto Assistant: memory.cpersona, tool.cron, tool.terminal, tool.websearch, tool.research, tool.agent_utils
- Cydonia 24B v4.3 (TheDrummer) Q4_K_M Ollama model support with ChatML template

### Fixed
- Default engine routing: Cloto Assistant was incorrectly using mind.deepseek instead of mind.cerebras (migration WHERE condition bug)
- ONNX embedding server: missing `token_type_ids` input caused all-MiniLM-L6-v2 inference to fail, breaking memory recall
- Response latency reduced from ~7.4s to ~2s (engine fix + embedding fix)

### Changed
- Ollama default model changed from glm-4.7-flash to cydonia
- Code cleanup: reduced ~600 lines across DB layer, handlers, and docs

---

## [0.4.22] — 2026-03-03

### Added
- CFR (Cost-First Router): high-speed engine tries first, escalates to high-quality engine on `[[ESCALATE]]`
- Auto-fallback: retriable errors (429/5xx/connection) automatically switch to fallback engine
- Routing rule extensions: `cfr`, `escalate_to`, `fallback` fields (backward-compatible)
- Dashboard UI: CFR toggle, escalation target, fallback selector in routing rule builder

---

## [0.4.21] — 2026-03-03

### Added
- Command approval system: HITL gate for terminal commands (Yes/Trust/No)
  - Kernel intercepts `execute_command` before MCP dispatch (YOLO mode bypasses)
  - DB-persisted exact match trust ("Yes") + session-scoped command name trust ("Trust")
  - Inline approval card in chat with 60s countdown timer
  - Tauri OS notification when approval pending and user is away
  - API endpoints: `POST /api/commands/:id/{approve,trust,deny}`
  - `trusted_commands` DB table + `CommandApprovalRequested/Result` events

### Changed
- Chat persistence moved from frontend to kernel (backend-complete)
  - User messages persisted in `handle_message()` before processing
  - Agent responses persisted before SSE `ThoughtResponse` emission
  - Frontend `postChatMessage` calls removed (no more fire-and-forget)
- LLM error handling improved across all layers
  - L1 (Proxy): HTTP status → user-friendly message + error code (`auth_failed`, `rate_limited`, etc.)
  - L2 (MCP Python): `LlmApiError` class replaces raw `raise_for_status()`, structured error response
  - L3 (Kernel): `format_engine_error()` adds actionable guidance per error code
  - L4 (Dashboard): `[Error]` messages displayed as amber error cards instead of plain text
  - Internal URLs (`127.0.0.1:8082`) no longer exposed to users
- Reset button long-press reduced from 2s to 1.5s
- Thinking state recovery: 30s timeout + `visibilitychange` listener to handle missed SSE responses

---

## [0.4.20] — 2026-03-03

### Added
- Dashboard update checker: "Check for Updates" button in Settings → About
- GitHub API-based version comparison with release notes display
- "Update Now" via Tauri shell plugin (desktop mode only)
- Tauri Native Auto-Update design (integrated into `docs/INSTALLER_DISTRIBUTION.md` § 6) for future implementation

---

## [0.4.19] — 2026-03-03

### Changed
- Extract password verification helper to `handlers/utils.rs` (deduplicate 2x20-line blocks)
- Python MCP server factory: `create_llm_mcp_server()` + `load_llm_provider_config()` reduce cerebras/deepseek to ~27 lines each
- Split `AgentPluginWorkspace.tsx` into `AvatarSection`, `ProfileSection`, `ServerAccessSection` components

---

## [0.4.18] — 2026-03-03

### Changed
- Split monolithic `db.rs` (1,732 lines) into 7 domain modules (`db/{audit,permissions,chat,mcp,api_keys,cron,llm}.rs`)
- Extract `mcp_tool_validator.rs` (~200 lines) from `managers/mcp.rs`
- Centralize validation constants and MIME helpers into `handlers/utils.rs`
- Remove unused npm packages (`clsx`, `tailwind-merge`)
- Remove false-positive `#[allow(dead_code)]` annotations (7 items)
- Remove unused code: `Tick` variant, `selected_agent()`, `create_slow_plugin()`

### Added
- Multi-Agent Delegation design document (`docs/MULTI_AGENT_DESIGN.md`) for v0.5.x

---

## [0.4.17] — 2026-03-03

### Fixed
- Agent card buttons unclickable when avatar background is set (`pointer-events-none` on overlay image)

---

## [0.4.16] — 2026-03-03

### Added
- PaddleOCR hybrid vision: OCR + llava combined analysis with A/B test support (hybrid/vision/ocr modes)
- Agent card avatar background in agent selection screen (blurred, hover effect)
- Default agent protection: name, description, avatar changes blocked for Cloto Assistant

### Changed
- Unified grid background: all 6 screens use `InteractiveGrid` (Canvas) with bottom fade
- Agent config UI: larger avatar preview (96px), bigger buttons, Remove button with red tint
- Agent card buttons enlarged (text-xs, size-14 icons)
- Chat avatar icons fill parent container (size 32-40px with overflow-hidden)
- Sidebar avatar icons enlarged to 24px
- MCP server grant/revoke: one-click on row (no separate button needed)
- Cloto Assistant description updated to reflect full capabilities

### Fixed
- Avatar broken image after delete (local `hasAvatar` state tracking)
- Backend-injected metadata fields polluting save (has_avatar, avatar_description excluded)
- Agent ID sanitization: URL-unsafe characters replaced with underscore
- Duplicate `api` import in AgentTerminal

---

## [0.4.15] — 2026-03-02

### Added
- KS2.2 Phase 2: Vector embedding search (ONNX MiniLM, cosine similarity) activated via mcp.toml config
- KS2.2 Phase 3: LLM-powered memory extraction — profile fact mining and episode summarization via Cerebras
- Auto-download ONNX model on first embedding server startup
- Memory/episode delete API (`DELETE /api/memories/:id`, `DELETE /api/episodes/:id`)
- Memory Core dashboard: delete buttons on memory cards and episode entries
- Auto `update_profile` trigger after episode archival

### Fixed
- Tauri: `mcp.toml` not found due to absolute path fallback not resolving to project root
- Tauri: venv Python not resolved due to `detect_project_root` not shared across modules

---

## [0.4.14] — 2026-03-02

### Added
- Auto-setup MCP Python venv on first kernel startup (`mcp_venv.rs`)
- Auto-resolve `python` command to venv Python in MCP transport (no venv activation needed)
- Cerebras tool calling: `gpt-oss-120b` now exposes `think_with_tools`
- Missing `pyproject.toml` for ollama, websearch, research MCP servers

### Fixed
- Agents using Cerebras engine could not use MCP tools (terminal, etc.) due to `supports_tools=False`

---

## [0.4.13] — 2026-03-02

### Added
- Agent avatar: image upload/serve/delete API (`POST/GET/DELETE /api/agents/:id/avatar`)
- Avatar vision analysis: auto-analyze via vision.capture MCP, description injected into LLM system prompt
- Agent rename: editable name/description fields in agent settings UI
- Clipboard paste: Ctrl+V image attachment support in chat input
- Window maximize on startup (Tauri)
- DB migration: `avatar_path`, `avatar_description` columns on agents table

### Fixed
- Cursor dot remnant when mouse leaves window (add `mouseleave`/`blur` handlers)
- Mermaid diagram text visibility on GitHub dark theme (`color:#333`)

### Quality
- YOLO mode issues registered (bug-170, 171, 172)

---

## [0.4.8] — 2026-03-01

### Added
- Engine routing: rule-based 3-layer engine selection (override > routing rules > default)
- MCP access control: wire up `resolve_tool_access()` 3-level priority resolution
- Episode auto-archival: `maybe_archive_episode()` triggers after 10+ unarchived messages
- McpClient notification handling: Server→Kernel JSON-RPC notification support (MGP §13 foundation)
- CI: `verify-issues` job in GitHub Actions
- CI: Branch Protection with required status checks
- Discord Bridge design document (`docs/DISCORD_BRIDGE_DESIGN.md`)
- MGP spec §19.5 `transport_websocket` extension, §19.6 External Event Bridge Pattern

### Fixed
- XSS: DOMPurify sanitization on `dangerouslySetInnerHTML`
- API key storage moved from localStorage to sessionStorage
- Unsafe `any` types replaced with proper React event types
- JSON parse guard (`safeJsonParse`) in api.ts
- Error state exposed from useAgents hook
- All clippy errors resolved (18 fixes)
- Test baseline updated, dashboard `--passWithNoTests`

### Security
- `default_policy` changed from `opt-in` to `opt-out` for MCP servers
- `save_mcp_server()` preserves `default_policy` on reconnect
- rollup HIGH severity path traversal fix

---

## [0.2.0] — 2026-02-26 (β2)

> Theme: Bug fixes, security hardening, performance improvements, documentation, and refinements

### Bug Fixes

- Resolve all open issues in issue registry (115/115 closed)
- Update 5 obsolete bug entries referencing deleted components
- Add error context to test assertions (`unwrap()` → `expect()`)

### Code Quality

- Suppress `clippy::too_many_lines` for Tauri entry point
- All `cargo clippy --workspace` warnings resolved
- All 90 tests passing, 0 ignored

### Security

- Install and run `cargo audit` — 0 vulnerabilities, 16 warnings (all GTK3 indirect deps, Linux-only)

### Documentation

- Rewrite CHANGELOG to version-based format (Keep a Changelog)
- Add v0.2.0 release scope document
- Fix 12 HIGH, 14 MEDIUM documentation inconsistencies across 9 files
- Align ARCHITECTURE.md, DEVELOPMENT.md, PROJECT_VISION.md with MCP-only architecture
- Update SCHEMA.md with 3 missing tables (runtime_plugins, revoked_keys, agent_plugins)
- Update MAINTAINABILITY.md metrics (crate count, file sizes, test count)
- Correct MCP server naming convention (core.cpersona → memory.cpersona)
- Clean up commit history (157 → 1 commit, author unified)

---

## [0.1.0] — 2026-02-26 (β1)

Initial release of ClotoCore — an AI agent orchestration platform built on
a Rust kernel with MCP-based plugin architecture.

### Core Architecture

- Event-driven Rust kernel with actor-model plugin system
- MCP (Model Context Protocol) as the sole plugin interface
- 5 MCP servers: Cerebras, CPersona Memory, DeepSeek, Embedding, Terminal
- ConsensusOrchestrator for multi-engine LLM coordination
- SQLite persistence with 24 migrations
- Rate limiting, audit logging, and permission isolation

### Dashboard

- React/TypeScript web UI with dark mode
- Agent workspace with MemoryCore design language
- MCP server management UI (Master-Detail layout)
- Real-time SSE event monitoring
- API key management with backend validation and revocation
- Tauri desktop application (multi-platform)

### CLI

- Agent management (create, list, inspect, delete)
- TUI dashboard with ratatui
- Log viewer with SSE follow mode
- Permission management commands

### Agent System

- Per-agent plugin assignment with config-seeded defaults
- Agent lifecycle management (create, delete, default protection)
- Custom skill registration with tool schema support
- Permission enforcement (visibility, revocation, runtime checks)

### Security

- API key authentication with Argon2id hashing
- Key revocation system with SHA-256 tracking
- Path traversal prevention and input validation
- CORS configuration with explicit header allowlists
- Human-in-the-loop permission approval workflow

### Infrastructure

- GitHub Actions CI/CD pipeline (5-platform build)
- Windows GUI installer (Inno Setup) with Japanese localization
- Shell and PowerShell installers with version validation
- GitHub Pages landing page with OS auto-detection
- BSL 1.1 license (converts to MIT on 2028-02-14)
