//go:build unix

// End-to-end tests through the command line: the seal fixtures via the
// `seal` subcommands, and one install driven exactly as the kernel drives
// it — `fetch` then `materialize`, JSON on stdin, events on stdout.
package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/Cloto-dev/ClotoCore/tools/cloto-installer/internal/seal"
	"github.com/Cloto-dev/ClotoCore/tools/cloto-installer/internal/testhub"
)

func fixtures(t *testing.T) string {
	t.Helper()
	root, _ := filepath.Abs(filepath.Join("..", "..", "crates", "core", "tests", "fixtures", "seal"))
	if _, err := os.Stat(filepath.Join(root, "tree", "cases.json")); err != nil {
		t.Fatalf("seal fixtures not found at %s", root)
	}
	return root
}

func cli(t *testing.T, stdin string, args ...string) (code int, stdout, stderr string) {
	t.Helper()
	var out, errb bytes.Buffer
	code = run(args, strings.NewReader(stdin), &out, &errb)
	return code, out.String(), errb.String()
}

func TestVersionPrintsOneLine(t *testing.T) {
	code, out, _ := cli(t, "", "version")
	if code != 0 || !strings.HasPrefix(out, "cloto-installer dev commit=unknown go=") {
		t.Errorf("code=%d out=%q", code, out)
	}
}

func TestUnknownCommandFails(t *testing.T) {
	if code, _, _ := cli(t, "", "bogus"); code != exitError {
		t.Errorf("code=%d", code)
	}
	if code, _, _ := cli(t, ""); code != exitError {
		t.Errorf("no args: code=%d", code)
	}
}

func TestSealSubcommandsReproduceTheTreeFixtures(t *testing.T) {
	root := filepath.Join(fixtures(t), "tree")
	var cases struct {
		Mint   []struct{ Tree, Expected string } `json:"mint"`
		Verify []struct {
			Name, Tree, Seal, Expect string
			KeyHex                   *string `json:"key_hex"`
		} `json:"verify"`
		EntryPointSeal []struct{ File, Expected string } `json:"entry_point_seal"`
	}
	data, _ := os.ReadFile(filepath.Join(root, "cases.json"))
	if err := json.Unmarshal(data, &cases); err != nil {
		t.Fatal(err)
	}
	keyFile := filepath.Join(root, "key.hex")
	keyHex, _ := os.ReadFile(keyFile)
	key := strings.TrimSpace(string(keyHex))

	for _, m := range cases.Mint {
		code, out, errText := cli(t, "", "seal", "tree", filepath.Join(root, m.Tree), "--key-hex", key)
		if code != 0 || strings.TrimSpace(out) != m.Expected {
			t.Errorf("seal tree %s: code=%d out=%q err=%q", m.Tree, code, out, errText)
		}
	}
	// The manifest bytes, through the CLI.
	code, out, _ := cli(t, "", "seal", "manifest", filepath.Join(root, "base"))
	want, _ := os.ReadFile(filepath.Join(root, "base.manifest.txt"))
	if code != 0 || out != string(want) {
		t.Errorf("seal manifest: code=%d\n%s", code, out)
	}
	for _, v := range cases.Verify {
		k := key
		if v.KeyHex != nil {
			k = *v.KeyHex
		}
		code, out, _ := cli(t, "", "seal", "verify", filepath.Join(root, v.Tree), v.Seal, "--key-hex", k)
		switch v.Expect {
		case "match":
			if code != exitOK || strings.TrimSpace(out) != "match" {
				t.Errorf("%s: code=%d out=%q", v.Name, code, out)
			}
		case "mismatch":
			if code != exitNegative || strings.TrimSpace(out) != "mismatch" {
				t.Errorf("%s: code=%d out=%q", v.Name, code, out)
			}
		case "error":
			if code != exitError {
				t.Errorf("%s: code=%d out=%q", v.Name, code, out)
			}
		}
	}
	for _, e := range cases.EntryPointSeal {
		code, out, _ := cli(t, "", "seal", "entry-point", filepath.Join(root, filepath.FromSlash(e.File)), "--key-hex", key)
		if code != 0 || strings.TrimSpace(out) != e.Expected {
			t.Errorf("seal entry-point: code=%d out=%q", code, out)
		}
	}
	// A key file (raw bytes, as the kernel stores seal.key) works too.
	raw, _ := hex.DecodeString(key)
	rawFile := filepath.Join(t.TempDir(), "seal.key")
	if err := os.WriteFile(rawFile, raw, 0o600); err != nil {
		t.Fatal(err)
	}
	code, out, _ = cli(t, "", "seal", "tree", filepath.Join(root, "clean"), "--key-file", rawFile)
	if code != 0 || strings.TrimSpace(out) != cases.Mint[0].Expected {
		t.Errorf("--key-file: code=%d out=%q", code, out)
	}
}

func TestSealVerdictSubcommandReproducesTheCatalogFixtures(t *testing.T) {
	root := filepath.Join(fixtures(t), "catalog")
	var cases struct {
		JWKS  string `json:"jwks"`
		Cases []struct {
			Name                      string          `json:"name"`
			Entry                     json.RawMessage `json:"entry"`
			InstalledEntryPointSHA256 string          `json:"installed_entry_point_sha256"`
			HubKey                    *string         `json:"hub_key"`
			Expect                    struct {
				Verdict string  `json:"verdict"`
				Reason  *string `json:"reason"`
				Code    *string `json:"code"`
			} `json:"expect"`
		} `json:"cases"`
	}
	data, _ := os.ReadFile(filepath.Join(root, "cases.json"))
	if err := json.Unmarshal(data, &cases); err != nil {
		t.Fatal(err)
	}
	jwks, _ := os.ReadFile(filepath.Join(root, cases.JWKS))
	for _, c := range cases.Cases {
		in := map[string]any{
			"entry":                        c.Entry,
			"installed_entry_point_sha256": c.InstalledEntryPointSHA256,
			"jwks":                         nil,
		}
		if c.HubKey != nil {
			in["jwks"] = json.RawMessage(jwks)
		}
		stdin, _ := json.Marshal(in)
		code, out, errText := cli(t, string(stdin), "seal", "verdict")
		var got struct {
			seal.Verdict
			EntryPointRead bool `json:"entry_point_read"`
		}
		if err := json.Unmarshal([]byte(out), &got); err != nil {
			t.Fatalf("%s: code=%d out=%q err=%q", c.Name, code, out, errText)
		}
		if got.Kind != c.Expect.Verdict {
			t.Errorf("%s: got %s, want %s", c.Name, got.Kind, c.Expect.Verdict)
		}
		if c.Expect.Reason != nil && got.Reason != *c.Expect.Reason {
			t.Errorf("%s: reason %s, want %s", c.Name, got.Reason, *c.Expect.Reason)
		}
		if c.Expect.Code != nil && got.Code != *c.Expect.Code {
			t.Errorf("%s: code %s, want %s", c.Name, got.Code, *c.Expect.Code)
		}
		wantExit := exitOK
		if c.Expect.Verdict == "tamper" {
			wantExit = exitNegative
		}
		if code != wantExit {
			t.Errorf("%s: exit %d, want %d", c.Name, code, wantExit)
		}
		if c.InstalledEntryPointSHA256 == "never-read" && got.EntryPointRead {
			t.Errorf("%s: entry point was read", c.Name)
		}
	}
}

// One install as the kernel drives it: fetch, then materialize, each with
// its JSON on stdin and its events on stdout.
func TestFetchThenMaterializeThroughTheCommandLine(t *testing.T) {
	hub := testhub.New(t)
	archive := testhub.StandaloneArchive(testhub.ServerPy)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write(archive)
	}))
	defer server.Close()
	dataDir := t.TempDir()
	uv, uvLog := testhub.InstallFakeUV(t, dataDir, false)
	tmp := filepath.Join(dataDir, "tmp")
	if err := os.MkdirAll(tmp, 0o755); err != nil {
		t.Fatal(err)
	}
	archivePath := filepath.Join(tmp, "demo-raw-url.tar.gz")
	entry := hub.Entry(testhub.EntryOptions{ID: "demo", Version: "1.0.0", ServerPy: testhub.ServerPy, Archive: archive, URL: server.URL + "/dl/demo.tar.gz"})

	fetchIn, _ := json.Marshal(map[string]any{
		"entry":        json.RawMessage(entry),
		"archive_path": archivePath,
		"pinned_addrs": []string{strings.TrimPrefix(server.URL, "http://")},
	})
	code, out, errText := cli(t, string(fetchIn), "fetch")
	if code != exitOK {
		t.Fatalf("fetch: code=%d\n%s\n%s", code, out, errText)
	}
	lines := strings.Split(strings.TrimSpace(out), "\n")
	if !strings.Contains(lines[len(lines)-1], `"type":"Result"`) || !strings.Contains(lines[len(lines)-1], `"ok":true`) {
		t.Errorf("fetch result line: %s", lines[len(lines)-1])
	}
	if _, err := os.Stat(archivePath); err != nil {
		t.Fatal("archive not written")
	}

	sealKey := bytes.Repeat([]byte{7}, 32)
	matIn, _ := json.Marshal(map[string]any{
		"entry":        json.RawMessage(entry),
		"archive_path": archivePath,
		"servers_dir":  filepath.Join(dataDir, "mcp-servers"),
		"tmp_dir":      tmp,
		"uv":           uv,
		"venv_dir":     filepath.Join(dataDir, "mcp-servers", ".venv"),
		"seal_key_hex": hex.EncodeToString(sealKey),
		"jwks":         hub.JWKS,
	})
	code, out, errText = cli(t, string(matIn), "materialize")
	if code != exitOK {
		t.Fatalf("materialize: code=%d\n%s\n%s", code, out, errText)
	}
	lines = strings.Split(strings.TrimSpace(out), "\n")
	var res struct {
		Type      string `json:"type"`
		OK        bool   `json:"ok"`
		Installed bool   `json:"installed"`
		Seal      struct {
			Verdict   string `json:"verdict"`
			LocalSeal string `json:"local_seal"`
		} `json:"seal"`
	}
	if err := json.Unmarshal([]byte(lines[len(lines)-1]), &res); err != nil || res.Type != "Result" {
		t.Fatalf("materialize result line: %s", lines[len(lines)-1])
	}
	if !res.OK || !res.Installed || res.Seal.Verdict != "verified" {
		t.Errorf("result: %+v", res)
	}
	installDir := filepath.Join(dataDir, "mcp-servers", "demo")
	if ok, err := seal.VerifyTreeSeal(installDir, res.Seal.LocalSeal, sealKey); err != nil || !ok {
		t.Errorf("seal does not verify: ok=%v err=%v", ok, err)
	}
	if len(testhub.UVCalls(uvLog)) != 2 {
		t.Errorf("uv calls: %v", testhub.UVCalls(uvLog))
	}
	// Every event line is the kernel's event shape.
	for _, line := range lines[:len(lines)-1] {
		var ev struct {
			Type string `json:"type"`
		}
		if err := json.Unmarshal([]byte(line), &ev); err != nil || ev.Type == "" {
			t.Errorf("bad event line: %s", line)
		}
	}

	// A refusal exits 2 with a Result of ok=false; here the stage is run
	// again on an archive that no longer exists, which extraction reports
	// as its own error (as the kernel does) rather than a crash.
	code, out, _ = cli(t, string(matIn), "materialize")
	if code != exitNegative || !strings.Contains(out, `"type":"StepError","step":"extract"`) {
		t.Errorf("a missing archive is an extraction refusal: code=%d out=%s", code, out)
	}
	bad := strings.Replace(string(fetchIn), `"pinned_addrs"`, `"x"`, 1)
	if code, _, _ := cli(t, bad, "fetch"); code != exitError {
		t.Errorf("fetch without addresses: code=%d", code)
	}
}

func TestMaterializeRefusalExitsNegative(t *testing.T) {
	hub := testhub.New(t)
	archive := testhub.StandaloneArchive(testhub.ServerPy)
	dataDir := t.TempDir()
	uv, _ := testhub.InstallFakeUV(t, dataDir, true)
	tmp := filepath.Join(dataDir, "tmp")
	if err := os.MkdirAll(tmp, 0o755); err != nil {
		t.Fatal(err)
	}
	archivePath := filepath.Join(tmp, "demo-raw-url.tar.gz")
	if err := os.WriteFile(archivePath, archive, 0o644); err != nil {
		t.Fatal(err)
	}
	entry := hub.Entry(testhub.EntryOptions{ID: "demo", Version: "1.0.0", ServerPy: testhub.ServerPy, Archive: archive, URL: "https://hub.invalid/x"})
	matIn, _ := json.Marshal(map[string]any{
		"entry": json.RawMessage(entry), "archive_path": archivePath,
		"servers_dir": filepath.Join(dataDir, "mcp-servers"), "tmp_dir": tmp,
		"uv": uv, "venv_dir": filepath.Join(dataDir, "mcp-servers", ".venv"),
		"seal_key_hex": strings.Repeat("ab", 32), "jwks": hub.JWKS,
	})
	code, out, _ := cli(t, string(matIn), "materialize")
	if code != exitNegative || !strings.Contains(out, `"type":"StepError"`) || !strings.Contains(out, `"ok":false`) {
		t.Errorf("code=%d\n%s", code, out)
	}
}
