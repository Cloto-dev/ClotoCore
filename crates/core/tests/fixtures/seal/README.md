# Seal parity fixtures

Language-neutral inputs and recorded answers for the two integrity checks the
marketplace installer performs. Any implementation of the installer — in this
crate or elsewhere — must produce these exact answers, otherwise a server
verified on one side fails on the other. `tests/seal_parity_test.rs` holds
this crate to them.

The recorded values were produced by this crate's implementation. Update
them only when the seal format changes on purpose, never to make a failing
test pass. `.gitattributes` disables line-ending conversion under this
directory: the trees are hashed byte for byte.

## `tree/` — the local tree seal

`tree-sha256:HEX` is HMAC-SHA256 under the per-installation key (`key.hex`,
a test key) over the canonical manifest of the installed tree:

- one line per covered file: `<sha256 hex>` + two spaces + `<relative path>` + `\n`
- paths use `/` on every platform
- lines are sorted by path **components**, not by the joined string —
  `pkg/sub/x` sorts before `pkg-extra.txt` and `pkg.txt`
- excluded from the manifest: any path with a `__pycache__`, `.venv`, `.git`
  or `node_modules` component, a component ending in `.egg-info` or
  `.dist-info`, and any `.pyc` / `.pyo` file
- symlinks and special files are neither hashed nor followed

| tree | what it is |
| --- | --- |
| `clean/` | an install tree as extracted |
| `base/` | `clean/` after the server ran once: bytecode caches, packaging metadata, a nested virtualenv, `node_modules`. Seals identically to `clean/` |
| `tampered/` | `base/` with one byte changed in a covered file |
| `other/` | an unrelated tree, for the foreign-seal case |

`cases.json` lists the expected seal for each tree (`mint`), the exact
manifest bytes for `base/` (`base.manifest.txt`), verification outcomes
(`verify`: `match` / `mismatch` / `error` — a missing or malformed seal is an
error, never a pass), and the entry-point seal (`sha256:HEX`, HMAC over one
file) the installer falls back to when no install directory resolves.

## `catalog/` — the install-time verdict on a catalog entry

Before a server is registered, the installer decides from the catalog
entry's evidence whether to mint a local seal at the declared trust tier
(`verified`), register the server unsealed so it runs under the untrusted
profile (`unsealed`, with a reason), or refuse the install (`tamper`, with a
code). The rules are documented on `install_seal_verdict` in
`handlers/marketplace.rs`; in short, the signed message is

```
mgp-seal/v1\nconnector_id=<id>\nversion=<version>\nentry_point_sha256=<hex>\n
```

or, when the entry carries an `archive` block,

```
mgp-seal/v2\n...entry_point_sha256=<hex>\narchive_sha256=<lowercase hex>\narchive_length=<decimal>\n
```

and the Ed25519 signature in `signature_payload.ed25519.sig` (base64) must
verify under the JWKS key named by `key_id`.

`jwks.json` is the hub's published key set and the two base entries in
`cases.json` are live catalog entries, trimmed to the fields that matter for
the decision — so the positive cases exercise real hub signatures, not keys
minted by a test. The other cases are mutations of those entries.
`installed_entry_point_sha256` stands in for hashing the installed entry
point; the sentinel `never-read` marks cases where the file must not be
read at all.
