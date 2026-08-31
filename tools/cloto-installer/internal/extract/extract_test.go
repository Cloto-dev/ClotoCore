package extract

import (
	"archive/tar"
	"bytes"
	"compress/gzip"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

type entry struct {
	name string
	data []byte
	typ  byte
	link string
}

func file(name string, data string) entry {
	return entry{name: name, data: []byte(data), typ: tar.TypeReg}
}
func dir(name string) entry { return entry{name: name, typ: tar.TypeDir} }

// globalHeader is the `pax_global_header` entry `git archive` (and so
// GitHub and the hub) opens an archive with, carrying the source commit.
func globalHeader() entry { return entry{name: "pax_global_header", typ: tar.TypeXGlobalHeader} }

func tarball(t *testing.T, entries ...entry) string {
	t.Helper()
	var buf bytes.Buffer
	gz := gzip.NewWriter(&buf)
	tw := tar.NewWriter(gz)
	for _, e := range entries {
		hdr := &tar.Header{Name: e.name, Typeflag: e.typ, Mode: 0o644, Size: int64(len(e.data)), Linkname: e.link}
		if e.typ == tar.TypeDir {
			hdr.Mode = 0o755
		}
		if e.typ == tar.TypeXGlobalHeader {
			hdr.Mode = 0
			hdr.PAXRecords = map[string]string{"comment": "d2368156b0f34f1c7930400cb4d35ed77c2eafb3"}
		}
		if err := tw.WriteHeader(hdr); err != nil {
			t.Fatal(err)
		}
		if _, err := tw.Write(e.data); err != nil {
			t.Fatal(err)
		}
	}
	if err := tw.Close(); err != nil {
		t.Fatal(err)
	}
	if err := gz.Close(); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(t.TempDir(), "a.tar.gz")
	if err := os.WriteFile(path, buf.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
	return path
}

func exists(t *testing.T, path string) bool {
	t.Helper()
	_, err := os.Lstat(path)
	return err == nil
}

func TestStrippedRemovesASharedTopLevelDirectory(t *testing.T) {
	archive := tarball(t,
		dir("demo-1.0.0/"),
		file("demo-1.0.0/server.py", "print(1)"),
		file("demo-1.0.0/pkg/__init__.py", ""),
	)
	target := t.TempDir()
	if err := TarballStripped(archive, target, nil); err != nil {
		t.Fatal(err)
	}
	if !exists(t, filepath.Join(target, "server.py")) || !exists(t, filepath.Join(target, "pkg", "__init__.py")) {
		t.Error("tree not laid out under target with the prefix stripped")
	}
	if exists(t, filepath.Join(target, "demo-1.0.0")) {
		t.Error("prefix directory must not survive")
	}
}

func TestStrippedKeepsTheLayoutWhenThereIsNoSharedPrefix(t *testing.T) {
	archive := tarball(t, file("server.py", "x"), file("pkg/a.py", "y"))
	target := t.TempDir()
	if err := TarballStripped(archive, target, nil); err != nil {
		t.Fatal(err)
	}
	if !exists(t, filepath.Join(target, "server.py")) || !exists(t, filepath.Join(target, "pkg", "a.py")) {
		t.Error("root-level entries must extract as they are")
	}
	// Two distinct top-level directories are not a shared prefix either.
	archive = tarball(t, file("a/x", "1"), file("b/y", "2"))
	target = t.TempDir()
	if err := TarballStripped(archive, target, nil); err != nil {
		t.Fatal(err)
	}
	if !exists(t, filepath.Join(target, "a", "x")) || !exists(t, filepath.Join(target, "b", "y")) {
		t.Error("split top-level directories must be kept")
	}
}

func TestSubdirSelectiveExtractsTheConnectorAndItsCommonSibling(t *testing.T) {
	archive := tarball(t,
		file("repo-v0/README.md", "readme"),
		file("repo-v0/servers/demo/server.py", "s"),
		file("repo-v0/servers/demo/pyproject.toml", "p"),
		file("repo-v0/servers/common/pyproject.toml", "p"),
		file("repo-v0/servers/common/common/__init__.py", ""),
		file("repo-v0/servers/other/server.py", "o"),
		file("repo-v0/servers/demonstration/server.py", "not the same directory"),
	)
	target := t.TempDir()
	if err := SubdirSelective(archive, target, "servers/demo", true, nil); err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{"servers/demo/server.py", "servers/common/common/__init__.py"} {
		if !exists(t, filepath.Join(target, filepath.FromSlash(want))) {
			t.Errorf("%s missing", want)
		}
	}
	for _, unwanted := range []string{"README.md", "servers/other", "servers/demonstration"} {
		if exists(t, filepath.Join(target, filepath.FromSlash(unwanted))) {
			t.Errorf("%s must not be extracted", unwanted)
		}
	}
	// Without the common flag the sibling stays out.
	target = t.TempDir()
	if err := SubdirSelective(archive, target, "servers/demo", false, nil); err != nil {
		t.Fatal(err)
	}
	if exists(t, filepath.Join(target, "servers", "common")) {
		t.Error("common extracted although not requested")
	}
}

func TestSubdirSelectiveFailsWhenTheSubdirIsEmpty(t *testing.T) {
	archive := tarball(t, file("repo-v0/servers/other/server.py", "o"))
	err := SubdirSelective(archive, t.TempDir(), "servers/demo", false, nil)
	if err == nil || !strings.Contains(err.Error(), "no files under subdir 'servers/demo'") {
		t.Errorf("got %v", err)
	}
}

func TestLinksAndSpecialFilesAreRefused(t *testing.T) {
	for _, typ := range []byte{tar.TypeSymlink, tar.TypeLink, tar.TypeChar, tar.TypeBlock, tar.TypeFifo} {
		archive := tarball(t, file("d/a.py", "x"), entry{name: "d/evil", typ: typ, link: "/etc/passwd"})
		target := t.TempDir()
		err := TarballStripped(archive, target, nil)
		if err == nil || !strings.Contains(err.Error(), "may not carry links or special files") {
			t.Errorf("type %q: got %v", typ, err)
		}
	}
}

func TestTraversalIsRefused(t *testing.T) {
	for _, name := range []string{"../escape.py", "d/../../escape.py", "/abs/escape.py"} {
		archive := tarball(t, file("d/a.py", "x"), file(name, "y"))
		target := t.TempDir()
		err := TarballStripped(archive, target, nil)
		if err == nil || !strings.Contains(err.Error(), "Path traversal detected") {
			t.Errorf("%s: got %v", name, err)
		}
		if exists(t, filepath.Join(filepath.Dir(target), "escape.py")) {
			t.Errorf("%s: escaped the target", name)
		}
	}
}

func TestEntryBudgetIsEnforced(t *testing.T) {
	b := newBudget()
	b.remainingEntries = 2
	warn := func(string) {}
	hdr := &tar.Header{Name: "x", Typeflag: tar.TypeReg}
	dest := filepath.Join(t.TempDir(), "x")
	for i := 0; i < 2; i++ {
		if _, err := writeEntry(hdr, strings.NewReader(""), dest, b, warn); err != nil {
			t.Fatal(err)
		}
	}
	if _, err := writeEntry(hdr, strings.NewReader(""), dest, b, warn); err == nil ||
		!strings.Contains(err.Error(), "more than 20000 entries") {
		t.Errorf("third entry: got %v", err)
	}
}

func TestByteBudgetDistinguishesExactFromOverrun(t *testing.T) {
	b := newBudget()
	b.remainingBytes = 4
	hdr := &tar.Header{Name: "x", Typeflag: tar.TypeReg}
	dest := filepath.Join(t.TempDir(), "x")
	if _, err := writeEntry(hdr, strings.NewReader("abcd"), dest, b, nil); err != nil {
		t.Fatalf("exactly the budget must pass: %v", err)
	}
	if b.remainingBytes != 0 {
		t.Errorf("budget not charged: %d", b.remainingBytes)
	}
	b.remainingBytes = 4
	if _, err := writeEntry(hdr, strings.NewReader("abcde"), dest, b, nil); err == nil ||
		!strings.Contains(err.Error(), "expands past") {
		t.Errorf("one byte over: got %v", err)
	}
}

func TestDuplicatePathsLastWins(t *testing.T) {
	archive := tarball(t, file("d/a.py", "first"), file("d/a.py", "second"))
	target := t.TempDir()
	if err := TarballStripped(archive, target, nil); err != nil {
		t.Fatal(err)
	}
	got, _ := os.ReadFile(filepath.Join(target, "a.py"))
	if string(got) != "second" {
		t.Errorf("got %q", got)
	}
}

func TestComponentsFollowTheKernelsPathIterator(t *testing.T) {
	for name, want := range map[string][]string{
		"a/b/c":     {"a", "b", "c"},
		"a//b/./c/": {"a", "b", "c"},
		"./a/b":     {".", "a", "b"},
		"a/../b":    {"a", "..", "b"},
		"/a/b":      {"/", "a", "b"},
		"":          nil,
	} {
		got := components(name)
		if strings.Join(got, "|") != strings.Join(want, "|") {
			t.Errorf("components(%q) = %v, want %v", name, got, want)
		}
	}
}

// A `git archive` tarball opens with a global pax header naming the
// commit. The reader hands that header back as an entry of its own, and
// it must not count as a top-level file: the shared prefix is still
// stripped and the subdir is still found. Caught by the first real hub
// archive after the fixtures above — all written without one — passed.
func TestGlobalPaxHeaderDoesNotHideTheSharedPrefix(t *testing.T) {
	archive := tarball(t,
		globalHeader(),
		dir("repo-1.0/"),
		dir("repo-1.0/servers/"),
		dir("repo-1.0/servers/demo/"),
		file("repo-1.0/servers/demo/server.py", "print('hi')"),
	)

	stripped := t.TempDir()
	if err := TarballStripped(archive, stripped, nil); err != nil {
		t.Fatal(err)
	}
	if !exists(t, filepath.Join(stripped, "servers", "demo", "server.py")) {
		t.Fatal("stripped extraction did not remove the shared prefix past the global header")
	}
	if exists(t, filepath.Join(stripped, "pax_global_header")) {
		t.Fatal("the global header was written out as a file")
	}

	selective := t.TempDir()
	if err := SubdirSelective(archive, selective, "servers/demo", false, nil); err != nil {
		t.Fatalf("subdir extraction failed: %v", err)
	}
	if !exists(t, filepath.Join(selective, "servers", "demo", "server.py")) {
		t.Fatal("subdir extraction did not find the connector past the global header")
	}
}
