// Package extract materialises a gzipped tarball on disk under the
// installer's extraction policy: links and special files are refused,
// paths may not escape the target, and one archive may expand to a bounded
// number of bytes and entries.
package extract

import (
	"archive/tar"
	"compress/gzip"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
)

// MaxExtractedBytes caps what one archive may expand to on disk. A
// verified digest proves the bytes are the ones the hub signed; it says
// nothing about what they decompress to.
const MaxExtractedBytes uint64 = 512 * 1024 * 1024

// MaxExtractedEntries caps how many entries one archive may carry, which a
// size budget alone does not bound (millions of empty files).
const MaxExtractedEntries = 20_000

// Warn receives non-fatal notices (an unsupported entry type that was
// skipped).
type Warn func(msg string)

type budget struct {
	remainingBytes   uint64
	remainingEntries int
}

func newBudget() *budget {
	return &budget{remainingBytes: MaxExtractedBytes, remainingEntries: MaxExtractedEntries}
}

func (b *budget) chargeEntry() error {
	if b.remainingEntries == 0 {
		return fmt.Errorf("Refusing to extract: archive carries more than %d entries", MaxExtractedEntries)
	}
	b.remainingEntries--
	return nil
}

// components splits a tar entry name the way the kernel's path iterator
// does: `/`-separated, empty and `.` segments dropped except a leading
// `.`, `..` kept (so the traversal check sees it), and a leading `/` kept
// as its own marker.
func components(name string) []string {
	var out []string
	if strings.HasPrefix(name, "/") {
		out = append(out, "/")
	}
	for i, seg := range strings.Split(name, "/") {
		switch {
		case seg == "":
			continue
		case seg == "." && !(i == 0 && len(out) == 0):
			continue
		}
		out = append(out, seg)
	}
	return out
}

// startsWith reports whether path begins with prefix, component-wise.
func startsWith(path, prefix []string) bool {
	if len(prefix) > len(path) {
		return false
	}
	for i := range prefix {
		if path[i] != prefix[i] {
			return false
		}
	}
	return true
}

// destination joins target with the entry's components. A leading `/`
// marker makes the result absolute, which the traversal check then
// refuses — the kernel's join replaces the target the same way.
func destination(target string, rel []string) string {
	if len(rel) > 0 && rel[0] == "/" {
		return filepath.Join(append([]string{string(filepath.Separator)}, rel[1:]...)...)
	}
	return filepath.Join(append([]string{target}, rel...)...)
}

// validateDest refuses a destination that resolves outside target after
// lexical normalisation of `..` (zip-slip).
func validateDest(target, dest string) error {
	full := filepath.Clean(dest)
	rel, err := filepath.Rel(filepath.Clean(target), full)
	if err != nil || rel == ".." || strings.HasPrefix(rel, ".."+string(filepath.Separator)) {
		return fmt.Errorf("Path traversal detected: '%s' escapes target directory", dest)
	}
	return nil
}

func entryTypeName(t byte) string {
	switch t {
	case tar.TypeSymlink:
		return "Symlink"
	case tar.TypeLink:
		return "Link"
	case tar.TypeChar:
		return "Char"
	case tar.TypeBlock:
		return "Block"
	case tar.TypeFifo:
		return "Fifo"
	}
	return fmt.Sprintf("0x%02x", t)
}

// writeEntry materialises one entry at dest. Regular files are written
// (last one wins for duplicate paths, both charged), directories created,
// links and special files refused, anything else skipped with a warning.
// Returns whether a regular file was written.
func writeEntry(hdr *tar.Header, r io.Reader, dest string, b *budget, warn Warn) (bool, error) {
	if err := b.chargeEntry(); err != nil {
		return false, err
	}
	switch hdr.Typeflag {
	case tar.TypeDir:
		return false, os.MkdirAll(dest, 0o755)
	case tar.TypeReg, tar.TypeRegA, tar.TypeCont, tar.TypeGNUSparse:
	case tar.TypeSymlink, tar.TypeLink, tar.TypeChar, tar.TypeBlock, tar.TypeFifo:
		return false, fmt.Errorf(
			"Refusing to extract '%s': archives may not carry links or special files (entry type %s)",
			dest, entryTypeName(hdr.Typeflag))
	default:
		if warn != nil {
			warn(fmt.Sprintf("Skipping unsupported tar entry type %s at '%s'", entryTypeName(hdr.Typeflag), dest))
		}
		return false, nil
	}
	if err := os.MkdirAll(filepath.Dir(dest), 0o755); err != nil {
		return false, err
	}
	out, err := os.Create(dest)
	if err != nil {
		return false, err
	}
	// Read one byte past the remaining allowance so an archive that exactly
	// exhausts the budget stays distinguishable from one that overruns it.
	limit := b.remainingBytes
	written, err := io.CopyN(out, r, int64(limit)+1)
	closeErr := out.Close()
	if err != nil && !errors.Is(err, io.EOF) {
		return false, err
	}
	if closeErr != nil {
		return false, closeErr
	}
	if uint64(written) > limit {
		return false, fmt.Errorf(
			"Refusing to extract '%s': archive expands past the %d-byte limit", dest, MaxExtractedBytes)
	}
	b.remainingBytes -= uint64(written)
	return true, nil
}

func open(archive string) (*os.File, *tar.Reader, error) {
	f, err := os.Open(archive)
	if err != nil {
		return nil, nil, err
	}
	gz, err := gzip.NewReader(f)
	if err != nil {
		f.Close()
		return nil, nil, err
	}
	return f, tar.NewReader(gz), nil
}

// isMetadata reports a tar entry that describes the archive rather than a
// file in it. The reader folds per-file pax headers into the entry they
// belong to, but a *global* pax header (`pax_global_header`, which
// GitHub- and hub-served archives open with, carrying the source commit)
// is returned as an entry of its own — a top-level "file" that would
// otherwise defeat shared-prefix detection and be reported as unsupported.
func isMetadata(hdr *tar.Header) bool {
	return hdr.Typeflag == tar.TypeXGlobalHeader || hdr.Typeflag == tar.TypeXHeader
}

// detectSharedPrefix finds a single top-level directory every entry sits
// under (the GitHub archive convention). A top-level file, or a second
// distinct top-level component, means there is none; a prefix is only
// strippable when something lives under it.
func detectSharedPrefix(archive string) ([]string, error) {
	f, tr, err := open(archive)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	var prefix []string
	sawNested := false
	for {
		hdr, err := tr.Next()
		if errors.Is(err, io.EOF) {
			break
		}
		if err != nil {
			return nil, err
		}
		if isMetadata(hdr) {
			continue
		}
		parts := components(hdr.Name)
		if len(parts) == 0 {
			continue
		}
		first := parts[0]
		hasMore := len(parts) > 1
		if !hasMore && hdr.Typeflag != tar.TypeDir {
			return nil, nil
		}
		if prefix == nil {
			prefix = []string{first}
		} else if prefix[0] != first {
			return nil, nil
		}
		if hasMore {
			sawNested = true
		}
	}
	if !sawNested {
		return nil, nil
	}
	return prefix, nil
}

// TarballStripped extracts a `.tar.gz` into target, stripping a single
// shared top-level directory when the archive uses one.
func TarballStripped(archive, target string, warn Warn) error {
	prefix, err := detectSharedPrefix(archive)
	if err != nil {
		return err
	}
	f, tr, err := open(archive)
	if err != nil {
		return err
	}
	defer f.Close()
	b := newBudget()
	for {
		hdr, err := tr.Next()
		if errors.Is(err, io.EOF) {
			break
		}
		if err != nil {
			return err
		}
		if isMetadata(hdr) {
			continue
		}
		rel := components(hdr.Name)
		if prefix != nil {
			if !startsWith(rel, prefix) {
				continue
			}
			rel = rel[len(prefix):]
		}
		if len(rel) == 0 {
			continue
		}
		dest := destination(target, rel)
		if err := validateDest(target, dest); err != nil {
			return err
		}
		if _, err := writeEntry(hdr, tr, dest, b, warn); err != nil {
			return err
		}
	}
	return nil
}

// SubdirSelective extracts one subdirectory tree — plus, when includeCommon
// is set, its sibling `common/` package — from a tarball, preserving
// repo-relative paths under target after stripping a shared top-level
// prefix. The layout (`target/<subdir>/…` with `common` beside it) matches
// a nested git clone.
func SubdirSelective(archive, target, subdir string, includeCommon bool, warn Warn) error {
	prefix, err := detectSharedPrefix(archive)
	if err != nil {
		return err
	}
	subdirRoot := components(subdir)
	var commonRoot []string
	if len(subdirRoot) > 1 {
		commonRoot = append(append([]string{}, subdirRoot[:len(subdirRoot)-1]...), "common")
	} else {
		commonRoot = []string{"common"}
	}

	f, tr, err := open(archive)
	if err != nil {
		return err
	}
	defer f.Close()
	b := newBudget()
	extractedAny := false
	for {
		hdr, err := tr.Next()
		if errors.Is(err, io.EOF) {
			break
		}
		if err != nil {
			return err
		}
		if isMetadata(hdr) {
			continue
		}
		rel := components(hdr.Name)
		if prefix != nil {
			if !startsWith(rel, prefix) {
				continue
			}
			rel = rel[len(prefix):]
		}
		if len(rel) == 0 {
			continue
		}
		wanted := startsWith(rel, subdirRoot) || (includeCommon && startsWith(rel, commonRoot))
		if !wanted {
			continue
		}
		dest := destination(target, rel)
		if err := validateDest(target, dest); err != nil {
			return err
		}
		wrote, err := writeEntry(hdr, tr, dest, b, warn)
		if err != nil {
			return err
		}
		if wrote && startsWith(rel, subdirRoot) {
			extractedAny = true
		}
	}
	if !extractedAny {
		return fmt.Errorf("tarball contains no files under subdir '%s'", subdir)
	}
	return nil
}
