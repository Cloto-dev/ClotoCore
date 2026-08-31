// Package fetch is stage 1 of a `raw_url` install: download the archive
// over a connection pinned to addresses the kernel has already cleared,
// checking it against the signed archive binding (or, failing that, the
// catalog-served digest) while it streams.
package fetch

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	"strings"
	"time"

	"github.com/Cloto-dev/ClotoCore/tools/cloto-installer/internal/catalog"
	"github.com/Cloto-dev/ClotoCore/tools/cloto-installer/internal/events"
	"github.com/Cloto-dev/ClotoCore/tools/cloto-installer/internal/seal"
)

// DefaultTimeoutSecs bounds the whole download.
const DefaultTimeoutSecs = 120

// Input is what the kernel hands this stage.
type Input struct {
	Entry catalog.Entry `json:"entry"`
	// Where the verified archive is written.
	ArchivePath string `json:"archive_path"`
	// Addresses (`ip:port`) the connection may be made to. The kernel
	// resolves the URL's host and applies its private-address policy
	// before calling; this stage never resolves names itself and refuses
	// to run without at least one address.
	PinnedAddrs []string `json:"pinned_addrs"`
	TimeoutSecs int      `json:"timeout_secs"`
}

// Result is the stage's final line.
type Result struct {
	OK          bool   `json:"ok"`
	ArchivePath string `json:"archive_path,omitempty"`
	Length      uint64 `json:"length,omitempty"`
	SHA256      string `json:"sha256,omitempty"`
}

// ErrInput marks a malformed input (not a refusal the kernel can show the
// user, a bug in the caller).
var ErrInput = errors.New("invalid fetch input")

// Log receives lines the kernel would have written to its own log.
type Log func(level, msg string)

// checkSpec validates the source the way the catalog SDK does.
func checkSpec(spec *catalog.RawURLSpec) (*url.URL, string) {
	parsed, err := url.Parse(spec.URL)
	if err != nil {
		return nil, "raw_url url is not parseable"
	}
	if parsed.Scheme != "http" && parsed.Scheme != "https" {
		return nil, "raw_url scheme must be http or https"
	}
	if spec.SHA256 != nil {
		h := *spec.SHA256
		if len(h) != 64 || strings.IndexFunc(h, func(r rune) bool {
			return !((r >= '0' && r <= '9') || (r >= 'a' && r <= 'f') || (r >= 'A' && r <= 'F'))
		}) >= 0 {
			return nil, "raw_url sha256 must be 64 hex characters"
		}
	}
	return parsed, ""
}

// Run performs the download. It returns (true, nil) once archivePath
// holds a verified archive, (false, nil) after a StepError was emitted —
// nothing is left at archivePath in that case — and an error for an I/O
// failure or bad input.
func Run(in *Input, em *events.Emitter, log Log) (bool, error) {
	in.Entry.Normalize()
	spec := in.Entry.RawURL()
	if spec == nil {
		return false, fmt.Errorf("%w: entry has no raw_url source", ErrInput)
	}
	if in.ArchivePath == "" {
		return false, fmt.Errorf("%w: archive_path is required", ErrInput)
	}
	if len(in.PinnedAddrs) == 0 {
		return false, fmt.Errorf("%w: pinned_addrs must name at least one address the kernel has cleared", ErrInput)
	}
	timeout := time.Duration(in.TimeoutSecs) * time.Second
	if in.TimeoutSecs <= 0 {
		timeout = DefaultTimeoutSecs * time.Second
	}

	parsed, reason := checkSpec(spec)
	if reason != "" {
		em.StepError("download", "Invalid raw_url source: "+reason, false)
		return false, nil
	}
	if parsed.Hostname() == "" {
		em.StepError("download", "raw_url has no host", false)
		return false, nil
	}

	client := &http.Client{
		Timeout: timeout,
		// A 3xx is answered, not followed: a cleared host must not be able
		// to redirect the download somewhere the kernel never cleared.
		CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse },
		Transport: &http.Transport{
			Proxy: nil,
			DialContext: func(ctx context.Context, network, _ string) (net.Conn, error) {
				var last error
				d := &net.Dialer{Timeout: 30 * time.Second}
				for _, addr := range in.PinnedAddrs {
					conn, err := d.DialContext(ctx, network, addr)
					if err == nil {
						return conn, nil
					}
					last = err
				}
				return nil, last
			},
		},
	}
	req, err := http.NewRequest(http.MethodGet, parsed.String(), nil)
	if err != nil {
		return false, err
	}
	req.Header.Set("User-Agent", "ClotoCore")
	resp, err := client.Do(req)
	if err != nil {
		return false, err
	}
	defer resp.Body.Close()

	if resp.StatusCode < 200 || resp.StatusCode > 299 {
		em.StepError("download", fmt.Sprintf("HTTP %s from %s", resp.Status, spec.URL), true)
		return false, nil
	}

	// Prefer the signed digest over the catalog-served one. When both are
	// present they must agree: the served value contradicting what the hub
	// signed is itself the substitution this check exists to catch.
	var expectedSHA256 string
	var signedLength uint64
	hasSignedLength := false
	switch binding := seal.ReadArchiveBinding(&in.Entry); binding.State {
	case "bound":
		if spec.SHA256 != nil && !strings.EqualFold(strings.TrimSpace(*spec.SHA256), binding.SHA256) {
			log("error", fmt.Sprintf("%s: TAMPER SUSPECT (archive_digest_contradiction) — catalog serves sha256 %s, the seal signed %s",
				in.Entry.ID, *spec.SHA256, binding.SHA256))
			em.StepError("download", fmt.Sprintf("archive digest contradiction: catalog serves %s, the seal signed %s",
				*spec.SHA256, binding.SHA256), false)
			return false, nil
		}
		expectedSHA256 = binding.SHA256
		signedLength = binding.Length
		hasSignedLength = true
	default:
		// v1 seal or none: the catalog's unsigned claim, byte for byte as
		// this path has always checked it.
		if spec.SHA256 != nil {
			expectedSHA256 = *spec.SHA256
		}
	}

	var total uint64
	hasTotal := resp.ContentLength >= 0
	if hasTotal {
		total = uint64(resp.ContentLength)
	}
	// With a signed length, refuse before reading a byte when the server
	// announces a different size.
	if hasSignedLength && hasTotal && total != signedLength {
		log("error", fmt.Sprintf("%s: TAMPER SUSPECT (archive_length_mismatch) — seal signed %d bytes, server announces %d",
			in.Entry.ID, signedLength, total))
		em.StepError("download", fmt.Sprintf("archive length mismatch: seal signed %d bytes, server announces %d",
			signedLength, total), false)
		return false, nil
	}

	file, err := os.Create(in.ArchivePath)
	if err != nil {
		return false, err
	}
	remove := func(why string) {
		if err := os.Remove(in.ArchivePath); err != nil {
			log("warn", fmt.Sprintf("Failed to cleanup archive after %s %s: %v", why, in.ArchivePath, err))
		}
	}
	hasher := sha256.New()
	var downloaded uint64
	buf := make([]byte, 64*1024)
	for {
		n, readErr := resp.Body.Read(buf)
		if n > 0 {
			chunk := buf[:n]
			if _, err := file.Write(chunk); err != nil {
				file.Close()
				return false, err
			}
			if expectedSHA256 != "" {
				hasher.Write(chunk)
			}
			downloaded += uint64(n)
			// A lying or absent Content-Length must not turn into an
			// unbounded write: the signed length is the ceiling.
			if hasSignedLength && downloaded > signedLength {
				log("error", fmt.Sprintf("%s: TAMPER SUSPECT (archive_overrun) — body exceeded the signed length of %d bytes",
					in.Entry.ID, signedLength))
				file.Close()
				remove("overrun")
				em.StepError("download", fmt.Sprintf("archive exceeded the signed length of %d bytes", signedLength), false)
				return false, nil
			}
			if hasTotal {
				progress := float32(1)
				if total > 0 {
					progress = float32(float64(downloaded) / float64(total))
					if progress > 1 {
						progress = 1
					}
				}
				em.StepProgress("download", progress,
					fmt.Sprintf("%.1f / %.1f MB", float64(downloaded)/1048576.0, float64(total)/1048576.0))
			}
		}
		if readErr != nil {
			if errors.Is(readErr, io.EOF) {
				break
			}
			file.Close()
			return false, readErr
		}
	}
	if err := file.Close(); err != nil {
		return false, err
	}

	// A body shorter than the signed length is a mismatch too — caught by
	// the digest below, but worth naming for the operator.
	if hasSignedLength && downloaded != signedLength {
		log("error", fmt.Sprintf("%s: TAMPER SUSPECT (archive_length_mismatch) — seal signed %d bytes, received %d",
			in.Entry.ID, signedLength, downloaded))
		remove("length mismatch")
		em.StepError("download", fmt.Sprintf("archive length mismatch: seal signed %d bytes, received %d",
			signedLength, downloaded), false)
		return false, nil
	}

	actual := ""
	if expectedSHA256 != "" {
		actual = hex.EncodeToString(hasher.Sum(nil))
		if !strings.EqualFold(actual, expectedSHA256) {
			if hasSignedLength {
				log("error", fmt.Sprintf("%s: TAMPER SUSPECT (archive_digest_mismatch) — seal signed sha256 %s, downloaded archive hashes to %s",
					in.Entry.ID, expectedSHA256, actual))
			}
			remove("sha256 mismatch")
			em.StepError("download", fmt.Sprintf("sha256 mismatch: expected %s, got %s", expectedSHA256, actual), false)
			return false, nil
		}
	}

	em.StepComplete("download")
	em.Result(Result{OK: true, ArchivePath: in.ArchivePath, Length: downloaded, SHA256: strings.ToLower(actual)})
	return true, nil
}
