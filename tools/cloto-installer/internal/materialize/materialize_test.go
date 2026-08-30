//go:build unix

// Contract tests for stage 2, mirroring the kernel's characterization of
// its own install path stage by stage: the events, the on-disk layout, the
// `uv` invocations, what each failure leaves behind, and the seal
// decision. Where this stage deliberately differs (nothing survives a
// failed install; a tamper suspect is never swapped in), the test says so.
// The Python step is observed through a stand-in `uv`, which is why the
// file is Unix-only.
package materialize

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/Cloto-dev/ClotoCore/tools/cloto-installer/internal/catalog"
	"github.com/Cloto-dev/ClotoCore/tools/cloto-installer/internal/events"
	"github.com/Cloto-dev/ClotoCore/tools/cloto-installer/internal/seal"
	"github.com/Cloto-dev/ClotoCore/tools/cloto-installer/internal/testhub"
)

var sealKey = bytes.Repeat([]byte{0x42}, 32)

type harness struct {
	t       *testing.T
	hub     *testhub.Hub
	dataDir string
	uv      string
	uvLog   string
	out     bytes.Buffer
	logs    []string
}

func newHarness(t *testing.T, failPip bool) *harness {
	t.Helper()
	h := &harness{t: t, hub: testhub.New(t), dataDir: t.TempDir()}
	h.uv, h.uvLog = testhub.InstallFakeUV(t, h.dataDir, failPip)
	return h
}

func (h *harness) serversDir() string { return filepath.Join(h.dataDir, "mcp-servers") }
func (h *harness) tmpDir() string     { return filepath.Join(h.dataDir, "tmp") }
func (h *harness) venvDir() string    { return filepath.Join(h.serversDir(), ".venv") }

// stage places an archive where stage 1 would have left it.
func (h *harness) stage(id string, archive []byte) string {
	if err := os.MkdirAll(h.tmpDir(), 0o755); err != nil {
		h.t.Fatal(err)
	}
	path := filepath.Join(h.tmpDir(), id+"-raw-url.tar.gz")
	if err := os.WriteFile(path, archive, 0o644); err != nil {
		h.t.Fatal(err)
	}
	return path
}

func (h *harness) entry(o testhub.EntryOptions) catalog.Entry {
	if o.ID == "" {
		o.ID = "demo"
	}
	if o.Version == "" {
		o.Version = "1.0.0"
	}
	if o.ServerPy == nil {
		o.ServerPy = testhub.ServerPy
	}
	if o.URL == "" {
		o.URL = "https://hub.invalid/dl/demo.tar.gz"
	}
	var e catalog.Entry
	if err := json.Unmarshal(h.hub.Entry(o), &e); err != nil {
		h.t.Fatal(err)
	}
	return e
}

func (h *harness) input(e catalog.Entry, archivePath string) *Input {
	return &Input{
		Entry:       e,
		ArchivePath: archivePath,
		ServersDir:  h.serversDir(),
		TmpDir:      h.tmpDir(),
		UV:          h.uv,
		VenvDir:     h.venvDir(),
		SealKeyHex:  hex.EncodeToString(sealKey),
		JWKS:        h.hub.JWKS,
		InstallLog:  filepath.Join(h.dataDir, "logs", "install.log"),
	}
}

func (h *harness) run(in *Input) Result {
	h.out.Reset()
	if err := Run(in, events.New(&h.out), func(level, msg string) { h.logs = append(h.logs, level+": "+msg) }); err != nil {
		h.t.Fatalf("materialize: %v\nevents:\n%s", err, h.out.String())
	}
	var res Result
	lines := strings.Split(strings.TrimSpace(h.out.String()), "\n")
	if err := json.Unmarshal([]byte(lines[len(lines)-1]), &res); err != nil {
		h.t.Fatalf("last line is not a Result: %q", lines[len(lines)-1])
	}
	return res
}

// steps returns the emitted events as compact labels; StepProgress is
// dropped and the Result line is not an event.
func (h *harness) steps() []string {
	var out []string
	for _, line := range strings.Split(strings.TrimSpace(h.out.String()), "\n") {
		var ev map[string]any
		if err := json.Unmarshal([]byte(line), &ev); err != nil {
			h.t.Fatalf("bad event line %q: %v", line, err)
		}
		switch ev["type"] {
		case "StepProgress", "Result":
			continue
		case "StepStart":
			out = append(out, "start:"+ev["step"].(string))
		case "StepComplete":
			out = append(out, "complete:"+ev["step"].(string))
		case "StepError":
			kind := "fatal"
			if ev["recoverable"].(bool) {
				kind = "recoverable"
			}
			out = append(out, "error:"+ev["step"].(string)+":"+kind+":"+ev["error"].(string))
		case "ServerInstall":
			out = append(out, "install:"+ev["server_name"].(string)+":"+ev["status"].(string))
		}
	}
	return out
}

func exists(path string) bool {
	_, err := os.Lstat(path)
	return err == nil
}

func assertSteps(t *testing.T, got []string, want ...string) {
	t.Helper()
	if strings.Join(got, "\n") != strings.Join(want, "\n") {
		t.Errorf("events differ:\n--- got\n%s\n--- want\n%s", strings.Join(got, "\n"), strings.Join(want, "\n"))
	}
}

// ── happy paths ──────────────────────────────────────────────────────

func TestStandaloneArchiveIsExtractedBuiltSealedAndSwappedIn(t *testing.T) {
	h := newHarness(t, false)
	archive := testhub.StandaloneArchive(testhub.ServerPy)
	e := h.entry(testhub.EntryOptions{Archive: archive})
	res := h.run(h.input(e, h.stage("demo", archive)))

	assertSteps(t, h.steps(),
		"start:extract",
		"complete:extract",
		"start:install_deps",
		"install:Demo:installing",
		"install:Demo:installed",
		"complete:install_deps",
	)

	// On disk: the tree under mcp-servers/<id> with the archive's top-level
	// directory stripped; no staging directory and no archive left in tmp.
	installDir := filepath.Join(h.serversDir(), "demo")
	if got, _ := os.ReadFile(filepath.Join(installDir, "server.py")); !bytes.Equal(got, testhub.ServerPy) {
		t.Error("server.py not in place")
	}
	if !exists(filepath.Join(installDir, "pkg", "__init__.py")) {
		t.Error("nested file missing")
	}
	if exists(filepath.Join(h.tmpDir(), "demo-staging")) || exists(filepath.Join(h.tmpDir(), "demo-raw-url.tar.gz")) {
		t.Error("staging directory or archive left in tmp")
	}
	if entries, _ := os.ReadDir(h.tmpDir()); len(entries) != 0 {
		t.Errorf("tmp not empty: %v", entries)
	}

	// The Python environment step: the shared virtualenv is created since
	// it did not exist, then the *staged* tree is installed into it — the
	// swap happens only after the environment is built.
	staged := filepath.Join(h.tmpDir(), "demo-staging")
	want := []string{
		"venv --python 3.13 " + h.venvDir(),
		"pip install --no-progress --python " + filepath.Join(h.venvDir(), "bin", "python") + " " + staged,
	}
	if got := testhub.UVCalls(h.uvLog); strings.Join(got, "\n") != strings.Join(want, "\n") {
		t.Errorf("uv calls:\n--- got\n%s\n--- want\n%s", strings.Join(got, "\n"), strings.Join(want, "\n"))
	}

	// The result: everything the kernel needs to register the server, with
	// a local tree seal that verifies against the installed tree.
	if !res.OK || !res.Installed {
		t.Fatalf("result: %+v", res)
	}
	if res.InstallDir != installDir || res.ServerPath != installDir {
		t.Errorf("paths: %+v", res)
	}
	if res.Command != "python" || len(res.Args) != 1 || res.Args[0] != filepath.Join(installDir, "server.py") {
		t.Errorf("command: %s %v", res.Command, res.Args)
	}
	if res.Venv == nil || !res.Venv.Created || res.Venv.Dir != h.venvDir() {
		t.Errorf("venv: %+v", res.Venv)
	}
	if res.Seal == nil || res.Seal.Verdict != "verified" || !strings.HasPrefix(res.Seal.LocalSeal, "tree-sha256:") {
		t.Fatalf("seal: %+v", res.Seal)
	}
	if ok, err := seal.VerifyTreeSeal(installDir, res.Seal.LocalSeal, sealKey); err != nil || !ok {
		t.Errorf("local seal does not verify against the installed tree: ok=%v err=%v", ok, err)
	}
	if !exists(filepath.Join(h.dataDir, "logs", "install.log")) {
		t.Error("install log not written")
	}
}

func TestMonorepoSubdirArchiveKeepsRepoRelativeLayoutAndInstallsCommonFirst(t *testing.T) {
	h := newHarness(t, false)
	archive := testhub.Tarball(
		testhub.File{Name: "repo-v0/README.md", Data: []byte("readme")},
		testhub.File{Name: "repo-v0/servers/demo/server.py", Data: testhub.ServerPy},
		testhub.File{Name: "repo-v0/servers/demo/pyproject.toml", Data: testhub.Pyproject},
		testhub.File{Name: "repo-v0/servers/common/pyproject.toml", Data: testhub.Pyproject},
		testhub.File{Name: "repo-v0/servers/common/common/__init__.py", Data: []byte{}},
		testhub.File{Name: "repo-v0/servers/other/server.py", Data: []byte("print('other')")},
	)
	subdir := "servers/demo"
	e := h.entry(testhub.EntryOptions{Directory: "servers/demo", Archive: archive, Subdir: &subdir, Dependencies: []string{"common"}})
	res := h.run(h.input(e, h.stage("demo", archive)))
	if !res.OK || !res.Installed {
		t.Fatalf("result: %+v\n%v", res, h.steps())
	}

	// A multi-segment catalog `directory` collapses to its last component;
	// inside it the connector keeps its repo-relative path, with the
	// declared `common` sibling alongside and nothing else from the repo.
	installDir := filepath.Join(h.serversDir(), "demo")
	serverPath := filepath.Join(installDir, "servers", "demo")
	if !exists(filepath.Join(serverPath, "server.py")) || !exists(filepath.Join(installDir, "servers", "common", "common", "__init__.py")) {
		t.Error("connector or common missing")
	}
	if exists(filepath.Join(installDir, "README.md")) || exists(filepath.Join(installDir, "servers", "other")) || exists(filepath.Join(h.serversDir(), "servers")) {
		t.Error("unrelated repo content extracted")
	}
	assertSteps(t, h.steps(),
		"start:extract",
		"complete:extract",
		"start:install_deps",
		"install:common:installing",
		"install:common:installed",
		"install:Demo:installing",
		"install:Demo:installed",
		"complete:install_deps",
	)

	// `common` is installed into the virtualenv before the connector.
	staged := filepath.Join(h.tmpDir(), "demo-staging")
	python := filepath.Join(h.venvDir(), "bin", "python")
	var pip []string
	for _, c := range testhub.UVCalls(h.uvLog) {
		if strings.HasPrefix(c, "pip ") {
			pip = append(pip, c)
		}
	}
	want := []string{
		"pip install --no-progress --python " + python + " " + filepath.Join(staged, "servers", "common"),
		"pip install --no-progress --python " + python + " " + filepath.Join(staged, "servers", "demo"),
	}
	if strings.Join(pip, "\n") != strings.Join(want, "\n") {
		t.Errorf("pip calls:\n--- got\n%s\n--- want\n%s", strings.Join(pip, "\n"), strings.Join(want, "\n"))
	}
	if res.ServerPath != serverPath || res.Args[0] != filepath.Join(serverPath, "server.py") {
		t.Errorf("paths: %+v", res)
	}
	if res.Seal.Verdict != "verified" {
		t.Errorf("seal: %+v", res.Seal)
	}
	if ok, _ := seal.VerifyTreeSeal(installDir, res.Seal.LocalSeal, sealKey); !ok {
		t.Error("tree seal does not cover the installed tree")
	}
}

func TestReinstallingANewerVersionReplacesTheTreeWholesale(t *testing.T) {
	h := newHarness(t, false)
	v1 := testhub.StandaloneArchive(testhub.ServerPy)
	res := h.run(h.input(h.entry(testhub.EntryOptions{Archive: v1}), h.stage("demo", v1)))
	if !res.Installed {
		t.Fatalf("first install: %+v", res)
	}
	installDir := filepath.Join(h.serversDir(), "demo")
	if err := os.WriteFile(filepath.Join(installDir, "leftover.txt"), []byte("stale"), 0o644); err != nil {
		t.Fatal(err)
	}

	newPy := []byte("import sys\nsys.exit(0)  # v2\n")
	v2 := testhub.StandaloneArchive(newPy)
	res = h.run(h.input(h.entry(testhub.EntryOptions{Version: "2.0.0", ServerPy: newPy, Archive: v2}), h.stage("demo", v2)))
	if !res.Installed {
		t.Fatalf("second install: %+v", res)
	}
	if got, _ := os.ReadFile(filepath.Join(installDir, "server.py")); !bytes.Equal(got, newPy) {
		t.Error("server.py not replaced")
	}
	if exists(filepath.Join(installDir, "leftover.txt")) {
		t.Error("nothing from the previous install may survive")
	}
	// The virtualenv is created once; each install runs its own pip step.
	venvs, pips := 0, 0
	for _, c := range testhub.UVCalls(h.uvLog) {
		if strings.HasPrefix(c, "venv ") {
			venvs++
		}
		if strings.HasPrefix(c, "pip ") {
			pips++
		}
	}
	if venvs != 1 || pips != 2 {
		t.Errorf("venv=%d pip=%d", venvs, pips)
	}
	if res.Venv.Created {
		t.Error("second install must report the existing virtualenv")
	}
}

// ── failure paths: what is emitted, what is left behind ──────────────

// Unlike the kernel's own path, a dependency failure leaves *nothing*: the
// staged tree is discarded, the previous install (if any) is untouched,
// and the archive is removed.
func TestDependencyInstallFailureLeavesNothingBehind(t *testing.T) {
	h := newHarness(t, false)
	v1 := testhub.StandaloneArchive(testhub.ServerPy)
	if res := h.run(h.input(h.entry(testhub.EntryOptions{Archive: v1}), h.stage("demo", v1))); !res.Installed {
		t.Fatalf("first install: %+v", res)
	}
	installDir := filepath.Join(h.serversDir(), "demo")

	// Now every pip call fails.
	h.uv, h.uvLog = testhub.InstallFakeUV(t, h.dataDir, true)
	newPy := []byte("import sys\nsys.exit(0)  # v2\n")
	v2 := testhub.StandaloneArchive(newPy)
	res := h.run(h.input(h.entry(testhub.EntryOptions{Version: "2.0.0", ServerPy: newPy, Archive: v2}), h.stage("demo", v2)))
	if res.OK || res.Installed {
		t.Fatalf("result: %+v", res)
	}
	steps := h.steps()
	assertSteps(t, steps[:4], "start:extract", "complete:extract", "start:install_deps", "install:Demo:installing")
	if len(steps) != 5 || !strings.HasPrefix(steps[4], "error:install_deps:recoverable:uv pip install failed: ") ||
		!strings.Contains(steps[4], "simulated dependency failure") {
		t.Errorf("steps: %v", steps)
	}
	// The failure detail is wrapped once.
	if strings.Contains(steps[4], "failed: uv pip install failed:") {
		t.Errorf("detail wrapped twice: %s", steps[4])
	}
	if got, _ := os.ReadFile(filepath.Join(installDir, "server.py")); !bytes.Equal(got, testhub.ServerPy) {
		t.Error("the previous install must be intact")
	}
	if exists(filepath.Join(h.tmpDir(), "demo-staging")) || exists(filepath.Join(h.tmpDir(), "demo-raw-url.tar.gz")) {
		t.Error("staging or archive left behind")
	}
}

func TestExtractionFailureKeepsThePreviousInstall(t *testing.T) {
	h := newHarness(t, false)
	v1 := testhub.StandaloneArchive(testhub.ServerPy)
	if res := h.run(h.input(h.entry(testhub.EntryOptions{Archive: v1}), h.stage("demo", v1))); !res.Installed {
		t.Fatalf("first install: %+v", res)
	}
	// A subdir the archive does not contain.
	subdir := "servers/missing"
	bad := h.entry(testhub.EntryOptions{Version: "2.0.0", Archive: v1, Subdir: &subdir})
	res := h.run(h.input(bad, h.stage("demo", v1)))
	if res.OK {
		t.Fatalf("result: %+v", res)
	}
	steps := h.steps()
	if len(steps) != 2 || !strings.HasPrefix(steps[1], "error:extract:fatal:tarball contains no files under subdir") {
		t.Errorf("steps: %v", steps)
	}
	if !exists(filepath.Join(h.serversDir(), "demo", "server.py")) {
		t.Error("previous install must be intact")
	}
	if exists(filepath.Join(h.tmpDir(), "demo-staging")) || exists(filepath.Join(h.tmpDir(), "demo-raw-url.tar.gz")) {
		t.Error("staging or archive left behind")
	}
	if len(testhub.UVCalls(h.uvLog)) != 2 {
		t.Error("no uv call may follow a failed extraction")
	}
}

// A signed identity that does not match the served entry is a tamper
// suspect. The tree is fully built (the verdict comes after the
// environment step, as in the kernel) but is never swapped in.
func TestInvalidHubSignatureIsReportedAndNeverSwappedIn(t *testing.T) {
	h := newHarness(t, false)
	archive := testhub.StandaloneArchive(testhub.ServerPy)
	e := h.entry(testhub.EntryOptions{Archive: archive})
	e.Version = "1.0.1" // the signed identity says 1.0.0
	res := h.run(h.input(e, h.stage("demo", archive)))
	if !res.OK || res.Installed {
		t.Fatalf("result: %+v", res)
	}
	if res.Seal == nil || res.Seal.Verdict != "tamper" || res.Seal.Code != "signature_invalid" ||
		!strings.HasPrefix(res.Seal.Message, "Ed25519 seal verification failed for 'demo'") {
		t.Errorf("seal: %+v", res.Seal)
	}
	steps := h.steps()
	if steps[len(steps)-1] != "complete:install_deps" {
		t.Errorf("no error event belongs to this stage; the kernel reports the verdict: %v", steps)
	}
	if exists(filepath.Join(h.serversDir(), "demo")) || exists(filepath.Join(h.tmpDir(), "demo-staging")) {
		t.Error("a tamper suspect must not reach the servers root or stay staged")
	}
	if len(h.logs) == 0 || !strings.Contains(h.logs[len(h.logs)-1], "TAMPER SUSPECT (signature_invalid)") {
		t.Errorf("tamper not logged: %v", h.logs)
	}
}

func TestEntryPointHashMismatchIsAnIntegrityTamper(t *testing.T) {
	h := newHarness(t, false)
	// Signed for one server.py, archive carries another.
	archive := testhub.StandaloneArchive([]byte("import os\n"))
	e := h.entry(testhub.EntryOptions{Archive: archive, ServerPy: testhub.ServerPy})
	res := h.run(h.input(e, h.stage("demo", archive)))
	if res.Installed || res.Seal.Verdict != "tamper" || res.Seal.Code != "integrity_mismatch" {
		t.Errorf("result: %+v seal=%+v", res, res.Seal)
	}
}

func TestUnverifiableSignatureInstallsUnsealed(t *testing.T) {
	h := newHarness(t, false)
	archive := testhub.StandaloneArchive(testhub.ServerPy)
	in := h.input(h.entry(testhub.EntryOptions{Archive: archive}), h.stage("demo", archive))
	in.JWKS = json.RawMessage("null") // the hub key is unreachable
	res := h.run(in)
	if !res.OK || !res.Installed {
		t.Fatalf("result: %+v", res)
	}
	if res.Seal.Verdict != "unsealed" || res.Seal.Reason != "hub_key_unavailable" || res.Seal.LocalSeal != "" {
		t.Errorf("seal: %+v", res.Seal)
	}
	if !exists(filepath.Join(h.serversDir(), "demo", "server.py")) {
		t.Error("an unsealed install still lands on disk")
	}
}

func TestTraversalInTheCatalogDirectoryIsRefusedBeforeExtraction(t *testing.T) {
	h := newHarness(t, false)
	archive := testhub.StandaloneArchive(testhub.ServerPy)
	e := h.entry(testhub.EntryOptions{Archive: archive})
	e.Directory = ".."
	res := h.run(h.input(e, h.stage("demo", archive)))
	if res.OK {
		t.Fatalf("result: %+v", res)
	}
	steps := h.steps()
	if len(steps) != 2 || !strings.HasPrefix(steps[1], "error:extract:fatal:Refusing to install 'demo'") {
		t.Errorf("steps: %v", steps)
	}
	if entries, _ := os.ReadDir(h.serversDir()); len(entries) != 0 {
		t.Errorf("servers root touched: %v", entries)
	}
}

// ── Rust connectors ──────────────────────────────────────────────────

func TestRustConnectorIsBuiltWithCargoAndRegistersTheBinary(t *testing.T) {
	h := newHarness(t, false)
	cargo := testhub.InstallFakeCargo(t, h.dataDir, "mgp-demo", false)
	mainRs := []byte("fn main() {}\n")
	archive := testhub.Tarball(
		testhub.File{Name: "demo-1.0.0/Cargo.toml", Data: []byte("[package]\nname = \"demo\"\n")},
		testhub.File{Name: "demo-1.0.0/src/main.rs", Data: mainRs},
	)
	// The signed entry point for a Rust connector is the built binary.
	e := h.entry(testhub.EntryOptions{Directory: "demo", Archive: archive, Runtime: "rust", ServerPy: []byte("bin")})
	in := h.input(e, h.stage("demo", archive))
	in.Cargo = cargo
	in.UV, in.VenvDir = "", ""
	res := h.run(in)
	if !res.OK || !res.Installed {
		t.Fatalf("result: %+v\n%v", res, h.steps())
	}
	assertSteps(t, h.steps(),
		"start:extract",
		"complete:extract",
		"start:cargo_build",
		"complete:cargo_build",
		"install:Demo:installed",
	)
	bin := filepath.Join(h.serversDir(), "demo", "target", "release", "mgp-demo")
	if res.Command != bin || len(res.Args) != 0 || !exists(bin) {
		t.Errorf("command: %s %v", res.Command, res.Args)
	}
	if got, _ := os.ReadFile(filepath.Join(h.serversDir(), "demo", "Cargo.toml")); !strings.Contains(string(got), "[workspace]") {
		t.Error("Cargo.toml must be patched with [workspace]")
	}
	if res.Seal.Verdict != "verified" {
		t.Errorf("seal: %+v", res.Seal)
	}
}

func TestRustBuildFailureIsFatalWithTheCompilerErrors(t *testing.T) {
	h := newHarness(t, false)
	cargo := testhub.InstallFakeCargo(t, h.dataDir, "mgp-demo", true)
	archive := testhub.Tarball(testhub.File{Name: "demo-1.0.0/Cargo.toml", Data: []byte("[package]\n")})
	e := h.entry(testhub.EntryOptions{Directory: "demo", Archive: archive, Runtime: "rust"})
	in := h.input(e, h.stage("demo", archive))
	in.Cargo = cargo
	res := h.run(in)
	if res.OK {
		t.Fatalf("result: %+v", res)
	}
	steps := h.steps()
	if steps[len(steps)-1] != "error:cargo_build:fatal:error[E0425]: cannot find value" {
		t.Errorf("steps: %v", steps)
	}
	if exists(filepath.Join(h.serversDir(), "demo")) || exists(filepath.Join(h.tmpDir(), "demo-staging")) {
		t.Error("nothing may be left after a failed build")
	}
}
