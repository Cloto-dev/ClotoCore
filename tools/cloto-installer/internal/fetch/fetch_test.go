package fetch

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/Cloto-dev/ClotoCore/tools/cloto-installer/internal/catalog"
	"github.com/Cloto-dev/ClotoCore/tools/cloto-installer/internal/events"
	"github.com/Cloto-dev/ClotoCore/tools/cloto-installer/internal/testhub"
)

type harness struct {
	t       *testing.T
	hub     *testhub.Hub
	server  *httptest.Server
	mux     *http.ServeMux
	archive string
	out     bytes.Buffer
	logs    []string
}

func newHarness(t *testing.T) *harness {
	t.Helper()
	h := &harness{t: t, hub: testhub.New(t), mux: http.NewServeMux()}
	h.server = httptest.NewServer(h.mux)
	t.Cleanup(h.server.Close)
	h.archive = filepath.Join(t.TempDir(), "demo-raw-url.tar.gz")
	return h
}

func (h *harness) url(name string) string { return h.server.URL + "/dl/" + name }

func (h *harness) serve(name string, body []byte) {
	h.mux.HandleFunc("/dl/"+name, func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write(body)
	})
}

func (h *harness) entry(archive []byte, url string) catalog.Entry {
	raw := h.hub.Entry(testhub.EntryOptions{
		ID: "demo", Version: "1.0.0", ServerPy: testhub.ServerPy, Archive: archive, URL: url,
	})
	var e catalog.Entry
	if err := json.Unmarshal(raw, &e); err != nil {
		h.t.Fatal(err)
	}
	return e
}

func (h *harness) run(e catalog.Entry) (bool, error) {
	in := &Input{
		Entry:       e,
		ArchivePath: h.archive,
		PinnedAddrs: []string{strings.TrimPrefix(h.server.URL, "http://")},
	}
	return Run(in, events.New(&h.out), func(level, msg string) { h.logs = append(h.logs, level+": "+msg) })
}

// steps returns the emitted events as compact labels; StepProgress is
// dropped because its count depends on chunking.
func (h *harness) steps() []string {
	var out []string
	for _, line := range strings.Split(strings.TrimSpace(h.out.String()), "\n") {
		if line == "" {
			continue
		}
		var ev map[string]any
		if err := json.Unmarshal([]byte(line), &ev); err != nil {
			h.t.Fatalf("bad event line %q: %v", line, err)
		}
		switch ev["type"] {
		case "StepProgress":
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
		case "Result":
			out = append(out, "result")
		default:
			out = append(out, ev["type"].(string))
		}
	}
	return out
}

func TestVerifiedArchiveIsWrittenAndReported(t *testing.T) {
	h := newHarness(t)
	archive := testhub.StandaloneArchive(testhub.ServerPy)
	h.serve("demo.tar.gz", archive)
	ok, err := h.run(h.entry(archive, h.url("demo.tar.gz")))
	if err != nil || !ok {
		t.Fatalf("ok=%v err=%v steps=%v", ok, err, h.steps())
	}
	got, err := os.ReadFile(h.archive)
	if err != nil || !bytes.Equal(got, archive) {
		t.Fatal("archive not written byte for byte")
	}
	steps := h.steps()
	if strings.Join(steps, ",") != "complete:download,result" {
		t.Errorf("steps: %v", steps)
	}
	if !strings.Contains(h.out.String(), `"sha256":"`+testhub.SHA256Hex(archive)+`"`) {
		t.Errorf("result lacks the digest: %s", h.out.String())
	}
}

func TestHTTPErrorIsRecoverableAndLeavesNothingBehind(t *testing.T) {
	h := newHarness(t)
	archive := testhub.StandaloneArchive(testhub.ServerPy)
	h.mux.HandleFunc("/dl/demo.tar.gz", func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(503)
	})
	ok, err := h.run(h.entry(archive, h.url("demo.tar.gz")))
	if err != nil || ok {
		t.Fatalf("ok=%v err=%v", ok, err)
	}
	steps := h.steps()
	if len(steps) != 1 || !strings.HasPrefix(steps[0], "error:download:recoverable:HTTP 503") {
		t.Errorf("steps: %v", steps)
	}
	if _, err := os.Stat(h.archive); err == nil {
		t.Error("archive must not exist after an HTTP error")
	}
}

func TestDigestMismatchIsFatalAndRemovesTheDownload(t *testing.T) {
	h := newHarness(t)
	archive := testhub.StandaloneArchive(testhub.ServerPy)
	substituted := append([]byte{}, archive...)
	substituted[len(substituted)-1] ^= 0xff
	h.serve("demo.tar.gz", substituted)
	ok, err := h.run(h.entry(archive, h.url("demo.tar.gz")))
	if err != nil || ok {
		t.Fatalf("ok=%v err=%v", ok, err)
	}
	steps := h.steps()
	if len(steps) != 1 || !strings.HasPrefix(steps[0], "error:download:fatal:sha256 mismatch:") {
		t.Errorf("steps: %v", steps)
	}
	if _, err := os.Stat(h.archive); err == nil {
		t.Error("archive must be removed after a digest mismatch")
	}
	if len(h.logs) == 0 || !strings.Contains(h.logs[0], "archive_digest_mismatch") {
		t.Errorf("tamper suspect not logged: %v", h.logs)
	}
}

func TestSignedLengthMismatchIsRefusedBeforeStreaming(t *testing.T) {
	h := newHarness(t)
	archive := testhub.StandaloneArchive(testhub.ServerPy)
	h.serve("demo.tar.gz", append(append([]byte{}, archive...), []byte("trailing garbage")...))
	ok, err := h.run(h.entry(archive, h.url("demo.tar.gz")))
	if err != nil || ok {
		t.Fatalf("ok=%v err=%v", ok, err)
	}
	steps := h.steps()
	if len(steps) != 1 || !strings.HasPrefix(steps[0], "error:download:fatal:archive length mismatch:") {
		t.Errorf("steps: %v", steps)
	}
	if _, err := os.Stat(h.archive); err == nil {
		t.Error("refused on the announced size: no archive file may exist")
	}
}

func TestOverrunWithoutContentLengthIsCaughtWhileStreaming(t *testing.T) {
	h := newHarness(t)
	archive := testhub.StandaloneArchive(testhub.ServerPy)
	h.mux.HandleFunc("/dl/demo.tar.gz", func(w http.ResponseWriter, _ *http.Request) {
		// Chunked: no Content-Length to refuse on, so the signed length is
		// the ceiling the stream is held to.
		w.Header().Set("Transfer-Encoding", "chunked")
		flusher, _ := w.(http.Flusher)
		_, _ = w.Write(archive)
		flusher.Flush()
		_, _ = w.Write([]byte("more than was signed"))
	})
	ok, err := h.run(h.entry(archive, h.url("demo.tar.gz")))
	if err != nil || ok {
		t.Fatalf("ok=%v err=%v", ok, err)
	}
	steps := h.steps()
	if len(steps) != 1 || !strings.HasPrefix(steps[0], "error:download:fatal:archive exceeded the signed length") {
		t.Errorf("steps: %v", steps)
	}
	if _, err := os.Stat(h.archive); err == nil {
		t.Error("archive must be removed after an overrun")
	}
}

func TestServedDigestContradictingTheSignedOneIsFatal(t *testing.T) {
	h := newHarness(t)
	archive := testhub.StandaloneArchive(testhub.ServerPy)
	h.serve("demo.tar.gz", archive)
	e := h.entry(archive, h.url("demo.tar.gz"))
	zeros := strings.Repeat("0", 64)
	e.Install.Source.SHA256 = &zeros
	ok, err := h.run(e)
	if err != nil || ok {
		t.Fatalf("ok=%v err=%v", ok, err)
	}
	steps := h.steps()
	if len(steps) != 1 || !strings.HasPrefix(steps[0], "error:download:fatal:archive digest contradiction:") {
		t.Errorf("steps: %v", steps)
	}
}

func TestRedirectsAreNotFollowed(t *testing.T) {
	h := newHarness(t)
	archive := testhub.StandaloneArchive(testhub.ServerPy)
	h.serve("real.tar.gz", archive)
	h.mux.HandleFunc("/dl/demo.tar.gz", func(w http.ResponseWriter, r *http.Request) {
		http.Redirect(w, r, "/dl/real.tar.gz", http.StatusFound)
	})
	ok, err := h.run(h.entry(archive, h.url("demo.tar.gz")))
	if err != nil || ok {
		t.Fatalf("ok=%v err=%v", ok, err)
	}
	steps := h.steps()
	if len(steps) != 1 || !strings.HasPrefix(steps[0], "error:download:recoverable:HTTP 302") {
		t.Errorf("steps: %v", steps)
	}
}

func TestMalformedSourceIsRefusedWithoutANetworkCall(t *testing.T) {
	h := newHarness(t)
	archive := testhub.StandaloneArchive(testhub.ServerPy)
	for name, url := range map[string]string{
		"scheme": "ftp://example.invalid/x.tar.gz",
		"host":   "http:///x.tar.gz",
	} {
		h.out.Reset()
		e := h.entry(archive, url)
		ok, err := h.run(e)
		if err != nil || ok {
			t.Fatalf("%s: ok=%v err=%v", name, ok, err)
		}
		steps := h.steps()
		if len(steps) != 1 || !strings.HasPrefix(steps[0], "error:download:fatal:") {
			t.Errorf("%s: steps %v", name, steps)
		}
	}
}

func TestAnAddressIsRequired(t *testing.T) {
	h := newHarness(t)
	archive := testhub.StandaloneArchive(testhub.ServerPy)
	in := &Input{Entry: h.entry(archive, h.url("demo.tar.gz")), ArchivePath: h.archive}
	if _, err := Run(in, events.New(&h.out), func(string, string) {}); err == nil {
		t.Error("fetch must refuse to run without pinned addresses")
	}
}
