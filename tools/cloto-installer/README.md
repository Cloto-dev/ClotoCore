# cloto-installer

The marketplace install engine the ClotoCore kernel runs as a subprocess.
It fetches a connector archive, verifies it against the catalog's signed
seal, extracts it, builds its environment (the shared Python virtualenv or
a cargo build) and decides the install-time seal. The kernel keeps what
touches its own state: request handling, the private-address policy for
downloads, provisioning `uv`, resolving the virtualenv, and the database
registration.

A static binary with no dependencies outside the Go standard library,
built for darwin/arm64, linux/amd64 and windows/amd64 and shipped beside
the kernel. The kernel checks `cloto-installer version` before use; a
missing or stale binary is reported, never worked around.

## Stages

A `raw_url` install is two stages, called in order. Each reads one JSON
document on stdin and writes progress events on stdout, one JSON object
per line, ending with a `Result` line. Events have the shape of the
kernel's setup progress events so the kernel forwards them unchanged:

```
{"type":"StepStart","step":"extract","description":"Extracting Demo"}
{"type":"StepProgress","step":"download","progress":0.5,"detail":"1.0 / 2.0 MB"}
{"type":"StepComplete","step":"extract"}
{"type":"StepError","step":"download","error":"HTTP 503 Service Unavailable from …","recoverable":true}
{"type":"ServerInstall","server_name":"Demo","status":"installing"}
{"type":"Result","ok":true, …}
```

Exit codes: `0` the stage completed with a positive answer; `2` it
completed with a negative one (a `StepError` was emitted; the kernel has
already been told why); `1` it could not run (bad input, I/O failure —
stderr carries the reason). Log lines the kernel would have written go to
stderr as `level: message`.

### `fetch`

```json
{
  "entry": { …catalog entry… },
  "archive_path": "/data/tmp/demo-raw-url.tar.gz",
  "pinned_addrs": ["203.0.113.10:443"],
  "timeout_secs": 120
}
```

Downloads `entry.install.source.url` over a connection made only to
`pinned_addrs` (the kernel resolves the host and applies its policy;
this stage never resolves names and refuses to run without an address),
with redirects answered rather than followed. The archive is checked while
it streams: against the signed `archive` binding when the entry carries a
`dual-v2` seal (digest and length — the announced size is refused up
front, an overrun is cut off mid-stream), otherwise against the
catalog-served `sha256`. A served digest that contradicts the signed one is
itself refused. Nothing is left at `archive_path` on a refusal.

Result: `{"ok":true,"archive_path":…,"length":…,"sha256":…}`.

### `materialize`

```json
{
  "entry": { …catalog entry… },
  "archive_path": "/data/tmp/demo-raw-url.tar.gz",
  "subdir": "servers/demo",
  "servers_dir": "/data/mcp-servers",
  "tmp_dir": "/data/tmp",
  "uv": "/data/bin/uv",
  "venv_dir": "/data/mcp-servers/.venv",
  "python_version": "3.13",
  "install_log": "/data/logs/install.log",
  "cargo": "cargo",
  "seal_key_hex": "…",
  "jwks": { "keys": [ … ] },
  "child_timeout_secs": 120,
  "build_timeout_secs": 600
}
```

`subdir` defaults to the entry's `raw_url` subdir. `jwks` is the hub's
key set, or `null` when it was unreachable (the entry then installs
unsealed). `seal_key_hex` is the per-installation seal key; the kernel
owns the key file.

The stage extracts into `{tmp_dir}/<install dir>-staging`, installs
`common` (when declared) and then the connector into the virtualenv from
the staged tree, decides the seal verdict on the staged entry point, mints
the local tree seal, and only then removes the previous install and renames
the staged tree into `{servers_dir}/<install dir>`. Staging until the end
is deliberate and differs from the kernel's own path: a dependency failure
or a tamper suspect leaves the previous install untouched and nothing new
on disk.

Result:

```json
{
  "ok": true,
  "installed": true,
  "install_dir": "/data/mcp-servers/demo",
  "server_path": "/data/mcp-servers/demo",
  "command": "python",
  "args": ["/data/mcp-servers/demo/server.py"],
  "venv": {"dir": "…", "python": "…", "created": true},
  "seal": {"verdict": "verified", "local_seal": "tree-sha256:…"}
}
```

`seal.verdict` is `verified` (with `local_seal`), `unsealed` (with a
`reason`: the kernel registers the server under the untrusted profile),
`tamper` (with a `code` and `message`: `installed` is false, nothing was
swapped in, and the kernel reports the refusal under its registration
step), or `error` (the tree could not be read; same handling). The
verdict is data, not an event, so the kernel's event sequence around
registration stays its own.

### `seal`

Direct access to the two integrity checks, for parity fixtures and
operators:

```
cloto-installer seal tree <root> (--key-hex H | --key-file F)
cloto-installer seal manifest <root>
cloto-installer seal verify <root> <seal> (--key-hex H | --key-file F)   exit 0 match / 2 mismatch / 1 error
cloto-installer seal entry-point <file> (--key-hex H | --key-file F)
cloto-installer seal verdict     stdin {entry, installed_entry_point_sha256 | installed_entry_point, jwks}
```

## Parity with the kernel

`crates/core/tests/fixtures/seal/` records the tree seals, manifest bytes
and catalog verdicts both implementations must produce; `go test ./...`
holds this binary to them (`internal/seal`), as `cargo test` holds the
kernel. The materialize tests mirror the kernel's characterization of its
install path stage by stage — events, on-disk layout, `uv` invocations,
what each failure leaves behind — and name each place this stage differs.

What is not carried over: the kernel's process-wide virtualenv lock (the
kernel holds it around the subprocess call), `uv` provisioning, and the
`git` / `pypi` / `docker` sources, which stay in the kernel.

## Building

```sh
cd tools/cloto-installer
go test ./...
CGO_ENABLED=0 go build -trimpath -ldflags "-s -w -X main.version=$V -X main.commit=$C" .
```

Cross-compile with `GOOS`/`GOARCH` (`darwin/arm64`, `linux/amd64`,
`windows/amd64`); no cgo, so every target builds from any host.
