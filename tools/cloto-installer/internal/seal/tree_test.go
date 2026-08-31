package seal

import (
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// fixtures is the language-neutral seal fixture set shared with the
// kernel's own parity test; the recorded values came from the kernel.
func fixtures(t *testing.T) string {
	t.Helper()
	root, err := filepath.Abs(filepath.Join("..", "..", "..", "..", "crates", "core", "tests", "fixtures", "seal"))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(root, "tree", "cases.json")); err != nil {
		t.Fatalf("seal fixtures not found at %s: %v", root, err)
	}
	return root
}

func readJSON(t *testing.T, path string, v any) {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(data, v); err != nil {
		t.Fatalf("parse %s: %v", path, err)
	}
}

type treeCases struct {
	KeyHex string `json:"key_hex"`
	Mint   []struct {
		Tree     string `json:"tree"`
		Expected string `json:"expected"`
	} `json:"mint"`
	Manifest []struct {
		Tree string `json:"tree"`
		File string `json:"file"`
	} `json:"manifest"`
	Verify []struct {
		Name   string  `json:"name"`
		Tree   string  `json:"tree"`
		Seal   string  `json:"seal"`
		KeyHex *string `json:"key_hex"`
		Expect string  `json:"expect"`
	} `json:"verify"`
	EntryPointSeal []struct {
		File     string `json:"file"`
		Expected string `json:"expected"`
	} `json:"entry_point_seal"`
}

func loadTreeCases(t *testing.T) (string, treeCases, []byte) {
	t.Helper()
	root := filepath.Join(fixtures(t), "tree")
	var cases treeCases
	readJSON(t, filepath.Join(root, "cases.json"), &cases)
	keyText, err := os.ReadFile(filepath.Join(root, cases.KeyHex))
	if err != nil {
		t.Fatal(err)
	}
	key, err := hex.DecodeString(strings.TrimSpace(string(keyText)))
	if err != nil {
		t.Fatal(err)
	}
	return root, cases, key
}

func TestTreeSealMintsTheRecordedValueForEveryFixtureTree(t *testing.T) {
	root, cases, key := loadTreeCases(t)
	if len(cases.Mint) < 3 {
		t.Fatal("fixture must cover several trees")
	}
	for _, m := range cases.Mint {
		got, err := ComputeTreeSeal(filepath.Join(root, m.Tree), key)
		if err != nil {
			t.Fatalf("tree %q: %v", m.Tree, err)
		}
		if got != m.Expected {
			t.Errorf("tree %q: got %s, want %s", m.Tree, got, m.Expected)
		}
	}
}

func TestGeneratedFilesArePresentInBaseAndDoNotDisturbTheSeal(t *testing.T) {
	root, cases, _ := loadTreeCases(t)
	sealOf := func(tree string) string {
		for _, m := range cases.Mint {
			if m.Tree == tree {
				return m.Expected
			}
		}
		t.Fatalf("mint case for %q", tree)
		return ""
	}
	if sealOf("base") != sealOf("clean") {
		t.Fatal("base and clean must seal identically")
	}
	base := filepath.Join(root, "base")
	for _, generated := range []string{
		"pkg/__pycache__/impl.cpython-312.pyc",
		"pkg/__pycache__/marker.txt",
		"stray.pyc",
		"stray.pyo",
		"demo.egg-info/PKG-INFO",
		"demo.dist-info/RECORD",
		"node_modules/dep/index.js",
		".venv/lib/sitecustomize.py",
	} {
		if info, err := os.Stat(filepath.Join(base, filepath.FromSlash(generated))); err != nil || !info.Mode().IsRegular() {
			t.Errorf("fixture lost its generated file %q", generated)
		}
	}
	manifest, err := TreeManifest(base)
	if err != nil {
		t.Fatal(err)
	}
	for _, excluded := range []string{"__pycache__", ".pyc", ".pyo", "egg-info", "dist-info", "node_modules", ".venv"} {
		if strings.Contains(manifest, excluded) {
			t.Errorf("%q leaked into the manifest:\n%s", excluded, manifest)
		}
	}
}

func TestTreeManifestMatchesTheRecordedBytes(t *testing.T) {
	root, cases, _ := loadTreeCases(t)
	if len(cases.Manifest) == 0 {
		t.Fatal("fixture records no manifest")
	}
	for _, m := range cases.Manifest {
		expected, err := os.ReadFile(filepath.Join(root, m.File))
		if err != nil {
			t.Fatal(err)
		}
		got, err := TreeManifest(filepath.Join(root, m.Tree))
		if err != nil {
			t.Fatal(err)
		}
		if got != string(expected) {
			t.Errorf("manifest of %q differs:\n--- got\n%s--- want\n%s", m.Tree, got, expected)
		}
	}
}

// Manifest lines are sorted by path components, so `pkg/sub/data.txt`
// sorts before `pkg-extra.txt` and `pkg.txt` even though `/` is a larger
// byte than `-` and `.`. Sorting the joined strings gives a different seal.
func TestManifestOrderIsByPathComponentNotByRawBytes(t *testing.T) {
	root, _, _ := loadTreeCases(t)
	manifest, err := TreeManifest(filepath.Join(root, "base"))
	if err != nil {
		t.Fatal(err)
	}
	lines := strings.Split(manifest, "\n")
	position := func(name string) int {
		for i, line := range lines {
			if strings.HasSuffix(line, "  "+name) {
				return i
			}
		}
		t.Fatalf("%q missing from manifest", name)
		return -1
	}
	if !(position("pkg/sub/data.txt") < position("pkg-extra.txt")) {
		t.Error("pkg/sub/data.txt must sort before pkg-extra.txt")
	}
	if !(position("pkg/sub/data.txt") < position("pkg.txt")) {
		t.Error("pkg/sub/data.txt must sort before pkg.txt")
	}
	// The raw-byte order really is the other way round.
	if !("pkg-extra.txt" < "pkg/sub/data.txt") || !("pkg.txt" < "pkg/sub/data.txt") {
		t.Fatal("the fixture names no longer distinguish the two orders")
	}
	if !(position("B.py") < position("_x.py")) || !(position("_x.py") < position("a.py")) {
		t.Error("uppercase and underscore must sort before lowercase")
	}
}

func TestTreeSealVerifyCasesGiveTheRecordedAnswer(t *testing.T) {
	root, cases, defaultKey := loadTreeCases(t)
	seen := map[string]bool{}
	for _, v := range cases.Verify {
		key := defaultKey
		if v.KeyHex != nil {
			k, err := hex.DecodeString(*v.KeyHex)
			if err != nil {
				t.Fatal(err)
			}
			key = k
		}
		ok, err := VerifyTreeSeal(filepath.Join(root, v.Tree), v.Seal, key)
		switch v.Expect {
		case "match":
			if err != nil || !ok {
				t.Errorf("%s: want match, got ok=%v err=%v", v.Name, ok, err)
			}
		case "mismatch":
			if err != nil || ok {
				t.Errorf("%s: want mismatch, got ok=%v err=%v", v.Name, ok, err)
			}
		case "error":
			if err == nil {
				t.Errorf("%s: want error, got ok=%v", v.Name, ok)
			}
		default:
			t.Fatalf("unknown expectation %q in %q", v.Expect, v.Name)
		}
		seen[v.Expect] = true
	}
	for _, want := range []string{"error", "match", "mismatch"} {
		if !seen[want] {
			t.Errorf("fixture exercises no %q case", want)
		}
	}
}

func TestEntryPointSealMatchesTheRecordedValue(t *testing.T) {
	root, cases, key := loadTreeCases(t)
	if len(cases.EntryPointSeal) == 0 {
		t.Fatal("fixture records no entry-point seal")
	}
	for _, e := range cases.EntryPointSeal {
		got, err := ComputeEntryPointSeal(filepath.Join(root, filepath.FromSlash(e.File)), key)
		if err != nil {
			t.Fatal(err)
		}
		if got != e.Expected {
			t.Errorf("entry-point seal of %q: got %s, want %s", e.File, got, e.Expected)
		}
	}
}

// A symlink is a leaf: never followed, never hashed. Extraction refuses to
// create one, so one in a tree arrived after install and must change the
// verdict (through the file set, not by reaching outside the tree).
func TestSymlinksAreNotFollowed(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "server.py"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	outside := t.TempDir()
	if err := os.WriteFile(filepath.Join(outside, "secret.py"), []byte("y"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(outside, filepath.Join(dir, "link")); err != nil {
		t.Skip("symlinks unavailable:", err)
	}
	manifest, err := TreeManifest(dir)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(manifest, "secret.py") || strings.Contains(manifest, "link") {
		t.Errorf("symlink was followed or hashed:\n%s", manifest)
	}
}

func TestExtensionFollowsTheKernelsRules(t *testing.T) {
	for name, want := range map[string]string{
		"a.pyc":   "pyc",
		".pyc":    "",
		".a.pyo":  "pyo",
		"noext":   "",
		"a.b.pyc": "pyc",
	} {
		if got := extension(name); got != want {
			t.Errorf("extension(%q) = %q, want %q", name, got, want)
		}
	}
}
