// Package testhub is test support: a stand-in hub that signs catalog
// entries the way the real one does, tarballs in the shape the hub serves,
// and a stand-in `uv` that records what it was asked to do. It is imported
// only by tests.
package testhub

import (
	"archive/tar"
	"bytes"
	"compress/gzip"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// KID is the hub key id used by every test entry.
const KID = "test-hub-key"

const ed25519Domain = "mgp-seal-ed25519-v1"

// Hub holds a signing key and its JWKS.
type Hub struct {
	priv ed25519.PrivateKey
	// JWKS is the `{"keys":[...]}` document naming KID.
	JWKS json.RawMessage
}

// New mints a hub keypair.
func New(t *testing.T) *Hub {
	t.Helper()
	pub, priv, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	jwks, _ := json.Marshal(map[string]any{"keys": []map[string]string{{
		"kty": "OKP", "crv": "Ed25519", "alg": "EdDSA", "use": "sig",
		"kid": KID, "x": base64.RawURLEncoding.EncodeToString(pub),
	}}})
	return &Hub{priv: priv, JWKS: jwks}
}

// Sign signs message under KID with the domain-separated input.
func (h *Hub) Sign(message []byte) string {
	input := append(append(append(append([]byte(ed25519Domain), 0), KID...), 0), message...)
	return base64.StdEncoding.EncodeToString(ed25519.Sign(h.priv, input))
}

// SHA256Hex hashes bytes.
func SHA256Hex(data []byte) string {
	sum := sha256.Sum256(data)
	return hex.EncodeToString(sum[:])
}

// EntryOptions shapes a signed catalog entry.
type EntryOptions struct {
	ID, Directory, Version string
	ServerPy               []byte
	Archive                []byte
	URL                    string
	Subdir                 *string
	Dependencies           []string
	Runtime                string
	BinName                *string
}

// Entry builds the JSON of a catalog entry the hub would publish for
// `Archive`: entry-point hash recorded, archive digest and length bound,
// Ed25519-signed (a `dual-v2` seal).
func (h *Hub) Entry(o EntryOptions) json.RawMessage {
	entryPointSHA := SHA256Hex(o.ServerPy)
	archiveSHA := SHA256Hex(o.Archive)
	canonical := []byte("mgp-seal/v2\nconnector_id=" + o.ID + "\nversion=" + o.Version +
		"\nentry_point_sha256=" + entryPointSHA + "\narchive_sha256=" + archiveSHA +
		"\narchive_length=" + fmt.Sprint(len(o.Archive)) + "\n")
	runtimeKind := o.Runtime
	if runtimeKind == "" {
		runtimeKind = "python"
	}
	deps := o.Dependencies
	if deps == nil {
		deps = []string{}
	}
	entry := map[string]any{
		"id": o.ID, "name": "Demo", "description": "demo connector", "category": "tool",
		"version": o.Version, "directory": o.Directory, "dependencies": deps,
		"env_vars": []any{}, "optional_env_vars": []any{}, "tags": []any{},
		"trust_level": "standard", "auto_restart": false, "icon": nil,
		"runtime": runtimeKind, "bin_name": o.BinName, "changelog": nil, "seal": nil,
		"entry_point_sha256": entryPointSHA,
		"signature_payload": map[string]any{
			"ed25519": map[string]any{"sig": h.Sign(canonical), "key_id": KID},
			"archive": map[string]any{"sha256": archiveSHA, "length": len(o.Archive)},
		},
		"install": map[string]any{
			"source":          map[string]any{"type": "raw_url", "url": o.URL, "sha256": archiveSHA, "subdir": o.Subdir},
			"package_manager": "uv",
		},
		"provider": nil,
	}
	data, err := json.Marshal(entry)
	if err != nil {
		panic(err)
	}
	return data
}

// File is one tarball entry.
type File struct {
	Name string
	Data []byte
}

// Tarball builds a gzipped tarball, GitHub-style (callers put every file
// under one shared top-level directory).
func Tarball(files ...File) []byte {
	var buf bytes.Buffer
	gz := gzip.NewWriter(&buf)
	tw := tar.NewWriter(gz)
	for _, f := range files {
		hdr := &tar.Header{Name: f.Name, Typeflag: tar.TypeReg, Mode: 0o644, Size: int64(len(f.Data))}
		if err := tw.WriteHeader(hdr); err != nil {
			panic(err)
		}
		if _, err := tw.Write(f.Data); err != nil {
			panic(err)
		}
	}
	if err := tw.Close(); err != nil {
		panic(err)
	}
	if err := gz.Close(); err != nil {
		panic(err)
	}
	return buf.Bytes()
}

// ServerPy and Pyproject are a minimal connector.
var (
	ServerPy  = []byte("import sys\nsys.exit(0)\n")
	Pyproject = []byte("[project]\nname = \"demo\"\nversion = \"0.1.0\"\n")
)

// StandaloneArchive is a single-connector archive with one shared prefix.
func StandaloneArchive(serverPy []byte) []byte {
	return Tarball(
		File{"demo-1.0.0/server.py", serverPy},
		File{"demo-1.0.0/pyproject.toml", Pyproject},
		File{"demo-1.0.0/pkg/__init__.py", []byte{}},
	)
}

// InstallFakeUV places a stand-in `uv` under `{dataDir}/bin/`. It appends
// every invocation's arguments to the returned log, creates a plausible
// virtualenv on `uv venv`, and, when failPip is set, fails every `uv pip`
// call. Unix only (it is a shell script).
func InstallFakeUV(t *testing.T, dataDir string, failPip bool) (uvPath, logPath string) {
	t.Helper()
	bin := filepath.Join(dataDir, "bin")
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatal(err)
	}
	logPath = filepath.Join(dataDir, "uv-calls.log")
	fail := ""
	if failPip {
		fail = "if [ \"$1\" = \"pip\" ]; then echo 'simulated dependency failure' >&2; exit 1; fi\n"
	}
	script := "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"" + logPath + "\"\n" +
		"if [ \"$1\" = \"venv\" ]; then mkdir -p \"$4/bin\" && printf 'version_info = 3.13.3\\n' > \"$4/pyvenv.cfg\"; fi\n" +
		fail + "exit 0\n"
	uvPath = filepath.Join(bin, "uv")
	if err := os.WriteFile(uvPath, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	return uvPath, logPath
}

// UVCalls reads the fake uv's log.
func UVCalls(logPath string) []string {
	data, err := os.ReadFile(logPath)
	if err != nil {
		return nil
	}
	return strings.Split(strings.TrimRight(string(data), "\n"), "\n")
}

// InstallFakeCargo places a stand-in `cargo` that creates
// `target/release/<bin>` in its working directory on `cargo build` and
// prints a Compiling line, or fails with an error line when failBuild is
// set. Unix only.
func InstallFakeCargo(t *testing.T, dataDir, binName string, failBuild bool) string {
	t.Helper()
	bin := filepath.Join(dataDir, "bin")
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatal(err)
	}
	var script string
	if failBuild {
		script = "#!/bin/sh\necho 'error[E0425]: cannot find value' >&2\nexit 101\n"
	} else {
		script = "#!/bin/sh\necho '   Compiling demo v0.1.0' >&2\nmkdir -p target/release && printf 'bin' > target/release/" + binName + "\nexit 0\n"
	}
	path := filepath.Join(bin, "cargo")
	if err := os.WriteFile(path, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	return path
}
