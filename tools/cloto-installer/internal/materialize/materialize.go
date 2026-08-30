// Package materialize is stage 2 of a `raw_url` install: extract the
// verified archive into a staging directory, build the connector's
// environment there (the shared Python virtualenv, or a cargo build),
// decide the install-time seal verdict, and only then swap the staged tree
// into the servers root. Nothing is written to the kernel's database here;
// the result carries what the kernel needs to register the server.
//
// Staging until the end is the property the kernel's own path lacked: a
// dependency failure or a tamper verdict leaves the previous install
// untouched and nothing new on disk.
package materialize

import (
	"bufio"
	"context"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"

	"github.com/Cloto-dev/ClotoCore/tools/cloto-installer/internal/catalog"
	"github.com/Cloto-dev/ClotoCore/tools/cloto-installer/internal/events"
	"github.com/Cloto-dev/ClotoCore/tools/cloto-installer/internal/extract"
	"github.com/Cloto-dev/ClotoCore/tools/cloto-installer/internal/seal"
)

// DefaultPythonVersion is the interpreter the shared virtualenv targets.
const DefaultPythonVersion = "3.13"

// DefaultChildTimeoutSecs bounds each `uv` invocation.
const DefaultChildTimeoutSecs = 120

// DefaultBuildTimeoutSecs bounds a `cargo build --release`.
const DefaultBuildTimeoutSecs = 600

// Input is what the kernel hands this stage.
type Input struct {
	Entry catalog.Entry `json:"entry"`
	// The archive stage 1 verified.
	ArchivePath string `json:"archive_path"`
	// The connector's directory inside a monorepo archive. Defaults to
	// the entry's `raw_url` subdir; an empty string means standalone.
	Subdir *string `json:"subdir"`
	// `{data_dir}/mcp-servers` and `{data_dir}/tmp`.
	ServersDir string `json:"servers_dir"`
	TmpDir     string `json:"tmp_dir"`
	// The `uv` binary; the kernel provisions it before calling.
	UV string `json:"uv"`
	// The shared virtualenv, resolved by the kernel: this stage never
	// searches for one.
	VenvDir       string `json:"venv_dir"`
	PythonVersion string `json:"python_version"`
	// Optional file `uv` output is appended to.
	InstallLog string `json:"install_log"`
	// `cargo` for Rust connectors; defaults to PATH lookup.
	Cargo string `json:"cargo"`
	// The per-installation seal key, hex. Required: a verified entry is
	// sealed before the result is written.
	SealKeyHex string `json:"seal_key_hex"`
	// The hub's JWKS document, or null when it was unreachable.
	JWKS             json.RawMessage `json:"jwks"`
	ChildTimeoutSecs int             `json:"child_timeout_secs"`
	BuildTimeoutSecs int             `json:"build_timeout_secs"`
}

// VenvState reports the Python environment used.
type VenvState struct {
	Dir    string `json:"dir"`
	Python string `json:"python"`
	// Whether this run created the virtualenv (it existed otherwise).
	Created bool `json:"created"`
}

// SealResult is the install-time verdict plus, when verified, the local
// seal minted over the installed tree.
type SealResult struct {
	// `verified`, `unsealed`, `tamper`, or `error` (the tree or entry
	// point could not be read; nothing was installed).
	Verdict   string `json:"verdict"`
	Reason    string `json:"reason,omitempty"`
	Code      string `json:"code,omitempty"`
	Message   string `json:"message,omitempty"`
	LocalSeal string `json:"local_seal,omitempty"`
}

// Result is the stage's final line.
type Result struct {
	// False when a StepError was emitted and the install stopped.
	OK bool `json:"ok"`
	// True when the tree is in place under the servers root. False with
	// OK=true means the verdict refused it (`tamper` / `error`): the
	// kernel reports that under its own registration step.
	Installed  bool        `json:"installed"`
	InstallDir string      `json:"install_dir,omitempty"`
	ServerPath string      `json:"server_path,omitempty"`
	Command    string      `json:"command,omitempty"`
	Args       []string    `json:"args,omitempty"`
	Venv       *VenvState  `json:"venv,omitempty"`
	Seal       *SealResult `json:"seal,omitempty"`
}

// ErrInput marks a malformed input.
var ErrInput = errors.New("invalid materialize input")

// Log receives lines the kernel would have written to its own log.
type Log func(level, msg string)

type run struct {
	in   *Input
	em   *events.Emitter
	log  Log
	name string
}

// Run performs the stage. It returns nil after writing a Result line
// (whether or not the install went through) and an error for an I/O
// failure or bad input, in which case no Result was written.
func Run(in *Input, em *events.Emitter, log Log) error {
	in.Entry.Normalize()
	if in.ArchivePath == "" || in.ServersDir == "" || in.TmpDir == "" {
		return fmt.Errorf("%w: archive_path, servers_dir and tmp_dir are required", ErrInput)
	}
	if in.SealKeyHex == "" {
		return fmt.Errorf("%w: seal_key_hex is required", ErrInput)
	}
	sealKey, err := hex.DecodeString(strings.TrimSpace(in.SealKeyHex))
	if err != nil || len(sealKey) == 0 {
		return fmt.Errorf("%w: seal_key_hex is not hex", ErrInput)
	}
	if !in.Entry.IsRust() && (in.UV == "" || in.VenvDir == "") {
		return fmt.Errorf("%w: uv and venv_dir are required for a Python connector", ErrInput)
	}
	if in.PythonVersion == "" {
		in.PythonVersion = DefaultPythonVersion
	}
	if in.ChildTimeoutSecs <= 0 {
		in.ChildTimeoutSecs = DefaultChildTimeoutSecs
	}
	if in.BuildTimeoutSecs <= 0 {
		in.BuildTimeoutSecs = DefaultBuildTimeoutSecs
	}
	if in.Cargo == "" {
		in.Cargo = "cargo"
	}
	r := &run{in: in, em: em, log: log, name: in.Entry.Name}
	return r.run(sealKey)
}

func (r *run) warn(msg string)   { r.log("warn", msg) }
func (r *run) info(msg string)   { r.log("info", msg) }
func (r *run) errorf(msg string) { r.log("error", msg) }

func (r *run) removeArchive() {
	if err := os.Remove(r.in.ArchivePath); err != nil {
		r.warn(fmt.Sprintf("Failed to cleanup archive %s: %v", r.in.ArchivePath, err))
	}
}

func (r *run) removeStaging(staging string) {
	if err := os.RemoveAll(staging); err != nil {
		r.warn(fmt.Sprintf("Failed to clear staging dir %s: %v", staging, err))
	}
}

func (r *run) run(sealKey []byte) error {
	in := r.in
	em := r.em
	entry := &in.Entry

	em.StepStart("extract", "Extracting "+r.name)
	if err := os.MkdirAll(in.ServersDir, 0o755); err != nil {
		return err
	}
	targetDir, err := entry.ResolveInstallDir(in.ServersDir)
	if err != nil {
		em.StepError("extract", err.Error(), false)
		em.Result(Result{OK: false})
		return nil
	}

	// Staging lives in the same `{data_dir}/tmp` as the download, so the
	// swap is a same-filesystem rename and a leftover never looks like an
	// installed server to the scans over the servers root.
	staging := filepath.Join(in.TmpDir, filepath.Base(targetDir)+"-staging")
	if _, err := os.Lstat(staging); err == nil {
		if err := os.RemoveAll(staging); err != nil {
			r.warn(fmt.Sprintf("Failed to clear stale staging dir %s: %v", staging, err))
		}
	}
	if err := os.MkdirAll(staging, 0o755); err != nil {
		return err
	}

	subdir := ""
	if in.Subdir != nil {
		subdir = *in.Subdir
	} else if spec := entry.RawURL(); spec != nil && spec.Subdir != nil {
		subdir = *spec.Subdir
	}
	subdir = strings.Trim(subdir, "/")
	needsCommon := entry.NeedsCommon()

	var extractErr error
	if subdir != "" {
		extractErr = extract.SubdirSelective(in.ArchivePath, staging, subdir, needsCommon, r.warn)
	} else {
		extractErr = extract.TarballStripped(in.ArchivePath, staging, r.warn)
	}
	if extractErr != nil {
		// The previous install is still intact — nothing has been touched.
		r.removeStaging(staging)
		r.removeArchive()
		em.StepError("extract", extractErr.Error(), false)
		em.Result(Result{OK: false})
		return nil
	}
	em.StepComplete("extract")

	stagedServer := staging
	finalServer := targetDir
	if subdir != "" {
		stagedServer = filepath.Join(staging, filepath.FromSlash(subdir))
		finalServer = filepath.Join(targetDir, filepath.FromSlash(subdir))
	}

	// Build the environment against the staged tree. The command the
	// kernel registers names the final location.
	var command string
	var args []string
	var stagedEntryPoint string
	var venv *VenvState
	if entry.IsRust() {
		ok, err := r.cargoBuild(stagedServer)
		if err != nil {
			return err
		}
		if !ok {
			r.removeStaging(staging)
			r.removeArchive()
			em.Result(Result{OK: false})
			return nil
		}
		binName := "mgp-" + entry.Directory
		if entry.BinName != nil {
			binName = *entry.BinName
		}
		if runtime.GOOS == "windows" {
			binName += ".exe"
		}
		stagedEntryPoint = filepath.Join(stagedServer, "target", "release", binName)
		if _, err := os.Stat(stagedEntryPoint); err != nil {
			em.StepError("cargo_build", "Binary not found at "+filepath.Join(finalServer, "target", "release", binName), false)
			r.removeStaging(staging)
			r.removeArchive()
			em.Result(Result{OK: false})
			return nil
		}
		em.ServerInstall(r.name, "installed")
		command = filepath.Join(finalServer, "target", "release", binName)
	} else {
		state, ok, err := r.pythonEnv(stagedServer, needsCommon)
		if err != nil {
			return err
		}
		if !ok {
			r.removeStaging(staging)
			r.removeArchive()
			em.Result(Result{OK: false})
			return nil
		}
		venv = state
		stagedEntryPoint = filepath.Join(stagedServer, "server.py")
		// Bare "python" is resolved to the virtualenv's interpreter by the
		// kernel at spawn time.
		command = "python"
		args = []string{filepath.Join(finalServer, "server.py")}
	}

	// The verdict is decided on the staged tree, before anything replaces
	// the previous install: a tamper suspect never reaches the servers root.
	sealResult := r.decide(stagedEntryPoint, staging, sealKey)
	if sealResult.Verdict == "tamper" || sealResult.Verdict == "error" {
		r.removeStaging(staging)
		r.removeArchive()
		em.Result(Result{OK: true, Installed: false, Venv: venv, Seal: sealResult})
		return nil
	}

	// Swap the staged tree in. The gap between removing the old tree and
	// renaming the new one is not itself atomic — a crash inside it leaves
	// the server uninstalled, which a reinstall fixes — but the staged tree
	// is complete and verified before the old one is touched.
	if _, err := os.Lstat(targetDir); err == nil {
		if err := os.RemoveAll(targetDir); err != nil {
			r.warn(fmt.Sprintf("Failed to clear existing target dir %s: %v", targetDir, err))
		}
	}
	if err := os.Rename(staging, targetDir); err != nil {
		return err
	}
	r.removeArchive()

	em.Result(Result{
		OK:         true,
		Installed:  true,
		InstallDir: targetDir,
		ServerPath: finalServer,
		Command:    command,
		Args:       args,
		Venv:       venv,
		Seal:       sealResult,
	})
	return nil
}

// decide computes the install-time verdict and, when verified, the local
// tree seal over the staged tree (relative paths only, so it is the seal of
// the tree at its final location too).
func (r *run) decide(stagedEntryPoint, staging string, sealKey []byte) *SealResult {
	entry := &r.in.Entry
	var hubKey = seal.KeyFromJWKS(r.in.JWKS, seal.SignatureKeyID(entry))
	hash := func() (string, error) {
		h, err := seal.FileSHA256(stagedEntryPoint)
		if err != nil {
			return "", fmt.Errorf("read entry point for integrity check (%s): %v", stagedEntryPoint, err)
		}
		return h, nil
	}
	verdict, err := seal.Decide(entry, hash, hubKey)
	if err != nil {
		return &SealResult{Verdict: "error", Message: err.Error()}
	}
	switch verdict.Kind {
	case "tamper":
		r.errorf(fmt.Sprintf("%s: TAMPER SUSPECT (%s) — %s", entry.ID, verdict.Code, verdict.Message))
		return &SealResult{Verdict: "tamper", Code: verdict.Code, Message: verdict.Message}
	case "unsealed":
		r.info(fmt.Sprintf("%s: registering unsealed (%s) — untrusted at spawn", entry.ID, verdict.Reason))
		return &SealResult{Verdict: "unsealed", Reason: verdict.Reason}
	}
	local, err := seal.ComputeTreeSeal(staging, sealKey)
	if err != nil {
		return &SealResult{Verdict: "error", Message: err.Error()}
	}
	r.info(fmt.Sprintf("%s: Ed25519 seal verified under hub key '%s'; minted local tree seal at declared tier '%s'",
		entry.ID, seal.SignatureKeyID(entry), entry.TrustLevel))
	return &SealResult{Verdict: "verified", LocalSeal: local}
}

// venvPython is the interpreter inside a virtualenv.
func venvPython(venvDir string) string {
	if runtime.GOOS == "windows" {
		return filepath.Join(venvDir, "Scripts", "python.exe")
	}
	return filepath.Join(venvDir, "bin", "python")
}

// resolveCommonSource locates an installable `common` package: beside the
// connector in a nested-clone layout, or flat under the servers root. Only
// a tree carrying `pyproject.toml` can be installed into the virtualenv.
func resolveCommonSource(serverPath, serversDir string) string {
	for _, candidate := range []string{
		filepath.Join(filepath.Dir(serverPath), "common"),
		filepath.Join(serversDir, "common"),
	} {
		if info, err := os.Stat(filepath.Join(candidate, "pyproject.toml")); err == nil && info.Mode().IsRegular() {
			return candidate
		}
	}
	return ""
}

// pythonEnv creates the shared virtualenv when it does not exist yet and
// installs `common` (when declared) then the connector into it. Returns
// ok=false after emitting a StepError.
func (r *run) pythonEnv(serverPath string, needsCommon bool) (*VenvState, bool, error) {
	in := r.in
	em := r.em
	em.StepStart("install_deps", "Installing "+r.name+" dependencies")

	timeout := time.Duration(in.ChildTimeoutSecs) * time.Second
	state := &VenvState{Dir: in.VenvDir, Python: venvPython(in.VenvDir)}
	if _, err := os.Stat(filepath.Join(in.VenvDir, "pyvenv.cfg")); err != nil {
		// The outcome is not checked here — as in the kernel, the pip step
		// below is what reports an unusable environment.
		_, _ = r.status(in.UV, []string{"venv", "--python", in.PythonVersion, in.VenvDir}, "", timeout)
		if _, err := os.Stat(filepath.Join(in.VenvDir, "pyvenv.cfg")); err == nil {
			state.Created = true
		}
	}

	if needsCommon {
		if common := resolveCommonSource(serverPath, in.ServersDir); common != "" {
			em.ServerInstall("common", "installing")
			ok, timedOut := r.status(in.UV, []string{"pip", "install", "--no-progress", "--python", state.Python, common}, "", timeout)
			switch {
			case ok:
				em.ServerInstall("common", "installed")
			case timedOut:
				r.warn(fmt.Sprintf("Common module install timed out (%ds)", in.ChildTimeoutSecs))
			default:
				r.warn("Failed to install common dependency")
			}
		} else {
			r.warn(fmt.Sprintf(
				"Connector '%s' declares a 'common' dependency but no installable common package (with pyproject.toml) was found next to %s or in %s — the server may fail with ModuleNotFoundError: No module named 'common'",
				in.Entry.ID, serverPath, in.ServersDir))
		}
	}

	em.ServerInstall(r.name, "installing")
	if detail := r.pipInstallStreaming(state.Python, serverPath, timeout); detail != "" {
		em.StepError("install_deps", "uv pip install failed: "+detail, true)
		return nil, false, nil
	}
	em.ServerInstall(r.name, "installed")
	em.StepComplete("install_deps")
	return state, true, nil
}

// status runs a command with no attached stdio and reports whether it
// exited successfully, and whether it was killed on timeout.
func (r *run) status(name string, args []string, dir string, timeout time.Duration) (ok bool, timedOut bool) {
	ctx, cancel := context.WithTimeout(context.Background(), timeout)
	defer cancel()
	cmd := exec.CommandContext(ctx, name, args...)
	cmd.Dir = dir
	cmd.Stdin = nil
	cmd.Stdout = nil
	cmd.Stderr = nil
	hideWindow(cmd)
	err := cmd.Run()
	if errors.Is(ctx.Err(), context.DeadlineExceeded) {
		return false, true
	}
	return err == nil, false
}

// pipInstallStreaming runs `uv pip install` for one tree, streaming its
// stderr as progress and into the install log. Returns "" on success or
// the failure detail.
func (r *run) pipInstallStreaming(python, serverPath string, timeout time.Duration) string {
	in := r.in
	ctx, cancel := context.WithTimeout(context.Background(), timeout)
	defer cancel()
	cmd := exec.CommandContext(ctx, in.UV, "pip", "install", "--no-progress", "--python", python, serverPath)
	cmd.Stdin = nil
	cmd.WaitDelay = 5 * time.Second
	hideWindow(cmd)
	stderr, err := cmd.StderrPipe()
	if err != nil {
		return "failed to run uv: " + err.Error()
	}
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return "failed to run uv: " + err.Error()
	}
	if err := cmd.Start(); err != nil {
		return "failed to run uv: " + err.Error()
	}

	var logFile *os.File
	if in.InstallLog != "" {
		_ = os.MkdirAll(filepath.Dir(in.InstallLog), 0o755)
		if f, err := os.OpenFile(in.InstallLog, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o644); err == nil {
			logFile = f
			defer f.Close()
		}
	}
	tail := make(chan []string, 1)
	go func() {
		var last []string
		scanner := bufio.NewScanner(stderr)
		scanner.Buffer(make([]byte, 0, 64*1024), 1024*1024)
		for scanner.Scan() {
			line := scanner.Text()
			if strings.TrimSpace(line) != "" {
				r.em.StepProgress("install_deps", -1, "["+r.name+"] "+line)
				if logFile != nil {
					ts := time.Now().Format("2006-01-02T15:04:05")
					_, _ = fmt.Fprintf(logFile, "[%s] [%s] %s\n", ts, r.name, line)
				}
			}
			last = append(last, line)
			if len(last) > 5 {
				last = last[1:]
			}
		}
		tail <- last
	}()
	go func() {
		scanner := bufio.NewScanner(stdout)
		scanner.Buffer(make([]byte, 0, 64*1024), 1024*1024)
		for scanner.Scan() {
		}
	}()

	err = cmd.Wait()
	last := <-tail
	if errors.Is(ctx.Err(), context.DeadlineExceeded) {
		return fmt.Sprintf("timed out (%ds)", in.ChildTimeoutSecs)
	}
	if err == nil {
		return ""
	}
	var exitErr *exec.ExitError
	if errors.As(err, &exitErr) {
		// The caller adds the "uv pip install failed:" prefix once.
		return strings.Join(last, " | ")
	}
	return "failed to wait for uv: " + err.Error()
}

// cargoBuild runs `cargo build --release` in the staged tree. Returns
// ok=false after emitting a StepError.
func (r *run) cargoBuild(serverPath string) (bool, error) {
	in := r.in
	em := r.em
	cargoToml := filepath.Join(serverPath, "Cargo.toml")
	if _, err := os.Stat(cargoToml); err != nil {
		r.warn("Extracted directory missing Cargo.toml: " + serverPath)
		em.StepError("cargo_build", fmt.Sprintf("Cargo.toml not found in %s. Extraction may have failed.", serverPath), false)
		return false, nil
	}
	// Ensure the package is not absorbed by a parent workspace.
	if content, err := os.ReadFile(cargoToml); err == nil {
		if !strings.Contains(string(content), "[workspace]") {
			if err := os.WriteFile(cargoToml, append(content, []byte("\n[workspace]\n")...), 0o644); err != nil {
				r.warn("Failed to patch Cargo.toml: " + err.Error())
			}
		}
	} else {
		r.warn(fmt.Sprintf("Failed to read Cargo.toml: %v", err))
	}
	r.info("Cargo.toml found, starting build in " + serverPath)

	em.StepStart("cargo_build", fmt.Sprintf("Building %s (this may take several minutes)", r.name))
	ctx, cancel := context.WithTimeout(context.Background(), time.Duration(in.BuildTimeoutSecs)*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, in.Cargo, "build", "--release")
	cmd.Dir = serverPath
	cmd.WaitDelay = 5 * time.Second
	stderr, err := cmd.StderrPipe()
	if err != nil {
		return false, err
	}
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return false, err
	}
	if err := cmd.Start(); err != nil {
		return false, err
	}
	errLines := make(chan []string, 1)
	go func() {
		var last []string
		scanner := bufio.NewScanner(stderr)
		scanner.Buffer(make([]byte, 0, 64*1024), 1024*1024)
		for scanner.Scan() {
			line := scanner.Text()
			if strings.Contains(line, "Compiling") || strings.Contains(line, "Downloading") {
				em.StepProgress("cargo_build", -1, line)
			} else if strings.Contains(line, "error") {
				last = append(last, line)
				if len(last) > 5 {
					last = last[1:]
				}
			}
		}
		errLines <- last
	}()
	go func() {
		scanner := bufio.NewScanner(stdout)
		scanner.Buffer(make([]byte, 0, 64*1024), 1024*1024)
		for scanner.Scan() {
		}
	}()
	err = cmd.Wait()
	last := <-errLines
	if errors.Is(ctx.Err(), context.DeadlineExceeded) {
		r.warn(fmt.Sprintf("cargo build --release timed out after %d minutes for %s", in.BuildTimeoutSecs/60, r.name))
		em.StepError("cargo_build", fmt.Sprintf("Build timed out after %d minutes", in.BuildTimeoutSecs/60), true)
		return false, nil
	}
	if err != nil {
		var exitErr *exec.ExitError
		if !errors.As(err, &exitErr) {
			r.warn(fmt.Sprintf("cargo build process error: %v", err))
			em.StepError("cargo_build", fmt.Sprintf("cargo build process error: %v", err), true)
			return false, nil
		}
		detail := "cargo build --release failed. Check Rust toolchain and dependencies."
		if len(last) > 0 {
			detail = strings.Join(last, "\n")
		}
		em.StepError("cargo_build", detail, false)
		return false, nil
	}
	em.StepComplete("cargo_build")
	return true, nil
}
