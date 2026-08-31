// Package seal implements the two integrity checks the installer performs
// and must agree on byte for byte with the kernel: the local tree seal
// minted over an installed server, and the install-time verdict on a
// catalog entry's signature. The fixtures under
// `crates/core/tests/fixtures/seal/` record the expected answers.
package seal

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

// TreeSealPrefix marks a seal as covering an installed tree rather than
// one entry point.
const TreeSealPrefix = "tree-sha256:"

// EntryPointSealPrefix marks a seal as HMAC over one file.
const EntryPointSealPrefix = "sha256:"

// MaxManifestFiles caps the files one manifest may cover. Install trees
// are source only (the Python environment is shared, not nested), so a
// tree this large means something unexpected is on disk.
const MaxManifestFiles = 20_000

// IsTreeSeal reports whether `seal` is a tree seal.
func IsTreeSeal(seal string) bool {
	return strings.HasPrefix(seal, TreeSealPrefix)
}

// isExcluded reports whether a relative path (as components) is left out
// of the manifest: anything produced by *running* the server after
// installation, so the tree does not fail its own seal on the second
// launch. Every component is checked, the file's own name included.
func isExcluded(components []string) bool {
	for _, c := range components {
		switch {
		case c == "__pycache__", c == ".venv", c == ".git", c == "node_modules":
			return true
		case strings.HasSuffix(c, ".egg-info"), strings.HasSuffix(c, ".dist-info"):
			return true
		}
	}
	switch extension(components[len(components)-1]) {
	case "pyc", "pyo":
		return true
	}
	return false
}

// extension is the part of a file name after its last dot, except that a
// name whose only dot is the leading one (`.pyc`) has no extension.
func extension(name string) string {
	i := strings.LastIndexByte(name, '.')
	if i <= 0 {
		return ""
	}
	return name[i+1:]
}

// lessByComponents orders relative paths component by component, each
// component by bytes — `pkg/sub/x` before `pkg-extra.txt` even though
// `/` is a larger byte than `-`.
func lessByComponents(a, b []string) bool {
	for i := 0; i < len(a) && i < len(b); i++ {
		if a[i] != b[i] {
			return a[i] < b[i]
		}
	}
	return len(a) < len(b)
}

// collectFiles lists every sealable file under root, as path components
// relative to root, sorted.
func collectFiles(root string) ([][]string, error) {
	var found [][]string
	stack := []string{root}
	for len(stack) > 0 {
		dir := stack[len(stack)-1]
		stack = stack[:len(stack)-1]
		entries, err := os.ReadDir(dir)
		if err != nil {
			return nil, fmt.Errorf("Failed to read directory: %s: %w", dir, err)
		}
		for _, entry := range entries {
			full := filepath.Join(dir, entry.Name())
			rel, err := filepath.Rel(root, full)
			if err != nil {
				continue
			}
			components := strings.Split(filepath.ToSlash(rel), "/")
			if isExcluded(components) {
				continue
			}
			// Lstat: a symlink is a leaf, never followed, so the manifest
			// cannot reach outside the tree.
			info, err := os.Lstat(full)
			if err != nil {
				return nil, fmt.Errorf("Failed to stat: %s: %w", full, err)
			}
			switch {
			case info.IsDir():
				stack = append(stack, full)
			case info.Mode().IsRegular():
				found = append(found, components)
				if len(found) > MaxManifestFiles {
					return nil, fmt.Errorf(
						"Refusing to seal '%s': more than %d files under the install tree",
						root, MaxManifestFiles)
				}
			}
			// Symlinks and special files are neither hashed nor walked.
		}
	}
	sort.Slice(found, func(i, j int) bool { return lessByComponents(found[i], found[j]) })
	return found, nil
}

// TreeManifest builds the canonical manifest for root: one
// `<sha256 hex>  <relative path>\n` line per covered file, sorted by path
// components, `/`-separated on every platform.
func TreeManifest(root string) (string, error) {
	files, err := collectFiles(root)
	if err != nil {
		return "", err
	}
	var b strings.Builder
	for _, components := range files {
		full := filepath.Join(root, filepath.Join(components...))
		data, err := os.ReadFile(full)
		if err != nil {
			return "", fmt.Errorf("Failed to read file for sealing: %s: %w", full, err)
		}
		sum := sha256.Sum256(data)
		b.WriteString(hex.EncodeToString(sum[:]))
		b.WriteString("  ")
		b.WriteString(strings.Join(components, "/"))
		b.WriteByte('\n')
	}
	return b.String(), nil
}

// ComputeTreeSeal mints the tree seal for the install tree at root.
func ComputeTreeSeal(root string, key []byte) (string, error) {
	info, err := os.Stat(root)
	if err != nil || !info.IsDir() {
		return "", fmt.Errorf("Cannot seal install tree: '%s' is not a directory", root)
	}
	manifest, err := TreeManifest(root)
	if err != nil {
		return "", err
	}
	mac := hmac.New(sha256.New, key)
	mac.Write([]byte(manifest))
	return TreeSealPrefix + hex.EncodeToString(mac.Sum(nil)), nil
}

// ErrNotTreeSeal is returned when a verification is asked of a value that
// is not a tree seal (an entry-point seal, an empty string).
var ErrNotTreeSeal = errors.New("not a tree seal")

// VerifyTreeSeal checks the tree at root against a previously minted
// seal. False means mismatch; an error means the tree could not be read
// or `expected` is not a tree seal at all — never a pass.
func VerifyTreeSeal(root, expected string, key []byte) (bool, error) {
	if !IsTreeSeal(expected) {
		return false, fmt.Errorf("%w: '%s'", ErrNotTreeSeal, expected)
	}
	computed, err := ComputeTreeSeal(root, key)
	if err != nil {
		return false, err
	}
	return computed == expected, nil
}

// ComputeEntryPointSeal mints the single-file seal (`sha256:HEX`, HMAC over
// the file's bytes) the kernel falls back to when no install tree
// resolves.
func ComputeEntryPointSeal(file string, key []byte) (string, error) {
	data, err := os.ReadFile(file)
	if err != nil {
		return "", fmt.Errorf("Failed to read file for sealing: %s: %w", file, err)
	}
	mac := hmac.New(sha256.New, key)
	mac.Write(data)
	return EntryPointSealPrefix + hex.EncodeToString(mac.Sum(nil)), nil
}

// FileSHA256 hashes one file, hex-encoded lower case.
func FileSHA256(file string) (string, error) {
	data, err := os.ReadFile(file)
	if err != nil {
		return "", err
	}
	sum := sha256.Sum256(data)
	return hex.EncodeToString(sum[:]), nil
}
