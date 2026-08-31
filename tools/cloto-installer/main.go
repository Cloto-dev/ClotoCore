// cloto-installer — the marketplace install engine the ClotoCore kernel
// runs as a subprocess. It fetches a connector archive, verifies it,
// extracts it, builds its environment and decides its seal; the kernel
// keeps the request handling and the database registration.
//
//	cloto-installer fetch          stdin: fetch input JSON   → stdout: progress events + Result
//	cloto-installer materialize    stdin: materialize input  → stdout: progress events + Result
//	cloto-installer seal tree <root> (--key-hex H | --key-file F)         → the tree seal
//	cloto-installer seal manifest <root>                                   → the canonical manifest
//	cloto-installer seal verify <root> <seal> (--key-hex H | --key-file F) → match / mismatch
//	cloto-installer seal entry-point <file> (--key-hex H | --key-file F)   → the entry-point seal
//	cloto-installer seal verdict   stdin: {entry, installed_entry_point_sha256, jwks} → verdict JSON
//	cloto-installer version
//
// Exit codes: 0 the stage completed and its answer is positive; 2 the stage
// completed with a negative answer (a StepError was emitted, a seal did not
// match); 1 the stage could not run (bad input, I/O failure).
//
// Progress events are one JSON object per line, shaped like the kernel's
// setup progress events (`{"type":"StepStart",...}`), ending with a line
// of `"type":"Result"`. Log lines go to stderr as `level: message`.
package main

import (
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"runtime"
	"strings"

	"github.com/Cloto-dev/ClotoCore/tools/cloto-installer/internal/catalog"
	"github.com/Cloto-dev/ClotoCore/tools/cloto-installer/internal/events"
	"github.com/Cloto-dev/ClotoCore/tools/cloto-installer/internal/fetch"
	"github.com/Cloto-dev/ClotoCore/tools/cloto-installer/internal/materialize"
	"github.com/Cloto-dev/ClotoCore/tools/cloto-installer/internal/seal"
)

// version and commit are stamped at build time
// (-ldflags "-X main.version=... -X main.commit=..."). The kernel checks
// `cloto-installer version` before use, so a stale or missing binary is
// reported rather than silently degrading the install path.
var (
	version = "dev"
	commit  = "unknown"
)

const (
	exitOK       = 0
	exitError    = 1
	exitNegative = 2
)

func main() {
	os.Exit(run(os.Args[1:], os.Stdin, os.Stdout, os.Stderr))
}

func usage(w io.Writer) {
	fmt.Fprint(w, `usage:
  cloto-installer fetch          (stdin: JSON)
  cloto-installer materialize    (stdin: JSON)
  cloto-installer seal tree <root> (--key-hex H | --key-file F)
  cloto-installer seal manifest <root>
  cloto-installer seal verify <root> <seal> (--key-hex H | --key-file F)
  cloto-installer seal entry-point <file> (--key-hex H | --key-file F)
  cloto-installer seal verdict   (stdin: JSON)
  cloto-installer version
`)
}

func run(args []string, stdin io.Reader, stdout, stderr io.Writer) int {
	if len(args) == 0 {
		usage(stderr)
		return exitError
	}
	log := func(level, msg string) {
		fmt.Fprintf(stderr, "%s: %s\n", level, msg)
	}
	switch args[0] {
	case "version":
		fmt.Fprintf(stdout, "cloto-installer %s commit=%s go=%s %s/%s\n",
			version, commit, runtime.Version(), runtime.GOOS, runtime.GOARCH)
		return exitOK
	case "fetch":
		var in fetch.Input
		if err := decode(stdin, &in); err != nil {
			log("error", err.Error())
			return exitError
		}
		em := events.New(stdout)
		ok, err := fetch.Run(&in, em, log)
		if err != nil {
			log("error", err.Error())
			return exitError
		}
		if !ok {
			em.Result(fetch.Result{OK: false})
			return exitNegative
		}
		return exitOK
	case "materialize":
		var in materialize.Input
		if err := decode(stdin, &in); err != nil {
			log("error", err.Error())
			return exitError
		}
		rec := &recordingWriter{w: stdout}
		em := events.New(rec)
		if err := materialize.Run(&in, em, log); err != nil {
			log("error", err.Error())
			return exitError
		}
		if !rec.lastOK {
			return exitNegative
		}
		return exitOK
	case "seal":
		return runSeal(args[1:], stdin, stdout, stderr)
	case "help", "-h", "--help":
		usage(stdout)
		return exitOK
	}
	usage(stderr)
	return exitError
}

func decode(r io.Reader, v any) error {
	dec := json.NewDecoder(r)
	dec.UseNumber()
	if err := dec.Decode(v); err != nil {
		return fmt.Errorf("stdin is not the expected JSON: %w", err)
	}
	return nil
}

// recordingWriter watches the event stream for the Result line so the exit
// code can reflect it without the stage reporting it twice.
type recordingWriter struct {
	w      io.Writer
	lastOK bool
}

func (r *recordingWriter) Write(p []byte) (int, error) {
	var probe struct {
		Type string `json:"type"`
		OK   bool   `json:"ok"`
	}
	if json.Unmarshal(p, &probe) == nil && probe.Type == "Result" {
		r.lastOK = probe.OK
	}
	return r.w.Write(p)
}

// keyFromFlags reads `--key-hex H` or `--key-file F` (raw bytes, as the
// kernel stores `seal.key`) from the remaining arguments.
func keyFromFlags(args []string) ([]byte, []string, error) {
	var key []byte
	var rest []string
	for i := 0; i < len(args); i++ {
		switch args[i] {
		case "--key-hex":
			if i+1 >= len(args) {
				return nil, nil, errors.New("--key-hex needs a value")
			}
			k, err := hex.DecodeString(strings.TrimSpace(args[i+1]))
			if err != nil {
				return nil, nil, fmt.Errorf("--key-hex is not hex: %w", err)
			}
			key = k
			i++
		case "--key-file":
			if i+1 >= len(args) {
				return nil, nil, errors.New("--key-file needs a value")
			}
			k, err := os.ReadFile(args[i+1])
			if err != nil {
				return nil, nil, err
			}
			key = k
			i++
		default:
			rest = append(rest, args[i])
		}
	}
	return key, rest, nil
}

func runSeal(args []string, stdin io.Reader, stdout, stderr io.Writer) int {
	if len(args) == 0 {
		usage(stderr)
		return exitError
	}
	fail := func(err error) int {
		fmt.Fprintf(stderr, "error: %v\n", err)
		return exitError
	}
	switch args[0] {
	case "tree", "verify", "entry-point":
		key, rest, err := keyFromFlags(args[1:])
		if err != nil {
			return fail(err)
		}
		if len(key) == 0 {
			return fail(errors.New("a key is required (--key-hex or --key-file)"))
		}
		switch args[0] {
		case "tree":
			if len(rest) != 1 {
				return fail(errors.New("seal tree takes exactly one root"))
			}
			s, err := seal.ComputeTreeSeal(rest[0], key)
			if err != nil {
				return fail(err)
			}
			fmt.Fprintln(stdout, s)
			return exitOK
		case "entry-point":
			if len(rest) != 1 {
				return fail(errors.New("seal entry-point takes exactly one file"))
			}
			s, err := seal.ComputeEntryPointSeal(rest[0], key)
			if err != nil {
				return fail(err)
			}
			fmt.Fprintln(stdout, s)
			return exitOK
		default:
			if len(rest) != 2 {
				return fail(errors.New("seal verify takes a root and a seal"))
			}
			ok, err := seal.VerifyTreeSeal(rest[0], rest[1], key)
			if err != nil {
				return fail(err)
			}
			if ok {
				fmt.Fprintln(stdout, "match")
				return exitOK
			}
			fmt.Fprintln(stdout, "mismatch")
			return exitNegative
		}
	case "manifest":
		if len(args) != 2 {
			return fail(errors.New("seal manifest takes exactly one root"))
		}
		m, err := seal.TreeManifest(args[1])
		if err != nil {
			return fail(err)
		}
		fmt.Fprint(stdout, m)
		return exitOK
	case "verdict":
		var in struct {
			Entry catalog.Entry `json:"entry"`
			// The installed entry point's hash, or a path to hash lazily.
			InstalledEntryPointSHA256 string          `json:"installed_entry_point_sha256"`
			InstalledEntryPoint       string          `json:"installed_entry_point"`
			JWKS                      json.RawMessage `json:"jwks"`
		}
		if err := decode(stdin, &in); err != nil {
			return fail(err)
		}
		in.Entry.Normalize()
		read := false
		hash := func() (string, error) {
			read = true
			if in.InstalledEntryPoint != "" {
				return seal.FileSHA256(in.InstalledEntryPoint)
			}
			return in.InstalledEntryPointSHA256, nil
		}
		hubKey := seal.KeyFromJWKS(in.JWKS, seal.SignatureKeyID(&in.Entry))
		v, err := seal.Decide(&in.Entry, hash, hubKey)
		if err != nil {
			return fail(err)
		}
		out := struct {
			seal.Verdict
			EntryPointRead bool `json:"entry_point_read"`
		}{v, read}
		data, _ := json.Marshal(out)
		fmt.Fprintln(stdout, string(data))
		if v.Kind == "tamper" {
			return exitNegative
		}
		return exitOK
	}
	usage(stderr)
	return exitError
}
