// Package catalog holds the slice of a catalog entry the installer reads
// and the rules that turn it into an on-disk location. Field names follow
// the catalog wire shape; fields the installer never consults are left to
// the kernel.
package catalog

import (
	"encoding/json"
	"fmt"
	"path/filepath"
	"runtime"
	"strings"
)

// Entry is a marketplace catalog entry, restricted to what an install
// needs. Unknown fields are ignored on decode.
type Entry struct {
	ID               string          `json:"id"`
	Name             string          `json:"name"`
	Version          string          `json:"version"`
	Directory        string          `json:"directory"`
	Dependencies     []string        `json:"dependencies"`
	TrustLevel       string          `json:"trust_level"`
	Runtime          string          `json:"runtime"`
	BinName          *string         `json:"bin_name"`
	EntryPointSHA256 *string         `json:"entry_point_sha256"`
	SignaturePayload json.RawMessage `json:"signature_payload"`
	Install          *InstallShape   `json:"install"`
}

// InstallShape carries the entry's source descriptor.
type InstallShape struct {
	Source Source `json:"source"`
}

// Source is the internally-tagged source descriptor (`type` selects the
// kind, the kind's fields sit beside it). Only `raw_url` is materialised
// by this installer; the other kinds stay with the kernel, so their
// fields are not modelled here.
type Source struct {
	Type   string  `json:"type"`
	URL    string  `json:"url"`
	SHA256 *string `json:"sha256"`
	Subdir *string `json:"subdir"`
}

// RawURLSpec is the `raw_url` source: a direct archive URL, its optional
// catalog-served digest, and the connector's directory inside a monorepo
// archive when the archive is one.
type RawURLSpec struct {
	URL    string
	SHA256 *string
	Subdir *string
}

// RawURL returns the entry's `raw_url` source, or nil when the entry has
// no install block or a source of another kind.
func (e *Entry) RawURL() *RawURLSpec {
	if e.Install == nil || e.Install.Source.Type != "raw_url" {
		return nil
	}
	src := e.Install.Source
	return &RawURLSpec{URL: src.URL, SHA256: src.SHA256, Subdir: src.Subdir}
}

// Normalize applies the catalog's defaults for fields that may be absent.
func (e *Entry) Normalize() {
	if e.Runtime == "" {
		e.Runtime = "python"
	}
	if e.TrustLevel == "" {
		e.TrustLevel = "standard"
	}
}

// IsRust reports whether the connector is built with cargo rather than
// installed into the shared Python environment.
func (e *Entry) IsRust() bool {
	return e.Runtime == "rust"
}

// NeedsCommon reports whether a Python connector declares the shared
// `common` package as a dependency.
func (e *Entry) NeedsCommon() bool {
	if e.IsRust() {
		return false
	}
	for _, d := range e.Dependencies {
		if d == "common" {
			return true
		}
	}
	return false
}

// SignatureField returns one top-level object of `signature_payload`
// (`ed25519`, `archive`), or nil when absent or not an object.
func (e *Entry) SignatureField(name string) map[string]json.RawMessage {
	if len(e.SignaturePayload) == 0 {
		return nil
	}
	var payload map[string]json.RawMessage
	if err := json.Unmarshal(e.SignaturePayload, &payload); err != nil {
		return nil
	}
	raw, ok := payload[name]
	if !ok {
		return nil
	}
	var obj map[string]json.RawMessage
	if err := json.Unmarshal(raw, &obj); err != nil {
		return nil
	}
	return obj
}

// EffectiveInstallDir is the directory name an install uses under the
// servers root. `directory` falls back to `id` when empty, and a value
// carrying path separators collapses to its last component so a
// monorepo-relative `directory` does not double the on-disk path.
func (e *Entry) EffectiveInstallDir() string {
	raw := e.Directory
	if raw == "" {
		raw = e.ID
	}
	trimmed := strings.Trim(raw, "/")
	last := trimmed
	if i := strings.LastIndexAny(trimmed, `/\`); i >= 0 {
		last = trimmed[i+1:]
	}
	if last == "" {
		return raw
	}
	return last
}

// ResolveInstallDir returns `serversDir/<install dir>`, refusing any value
// that is not exactly one ordinary path component: `..`, `.`, an absolute
// path, a multi-segment value, or the empty string. The catalog is served
// by the hub and can be stale or malicious; the install target does not
// exist yet, so this is a lexical guard rather than a filesystem one.
func (e *Entry) ResolveInstallDir(serversDir string) (string, error) {
	component := e.EffectiveInstallDir()
	if !isSingleNormalComponent(component) {
		return "", fmt.Errorf(
			"Refusing to install '%s': install directory '%s' is not a single path component under mcp-servers (path traversal blocked)",
			e.ID, component)
	}
	return filepath.Join(serversDir, component), nil
}

func isSingleNormalComponent(s string) bool {
	if s == "" || s == "." || s == ".." {
		return false
	}
	if strings.ContainsAny(s, `/\`) {
		return false
	}
	if runtime.GOOS == "windows" && filepath.VolumeName(s) != "" {
		return false
	}
	return true
}
