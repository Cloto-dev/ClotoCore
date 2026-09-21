# Hub Access Tokens — Kernel Side

Status: implemented (0.7.0 line).
Scope: how the kernel holds and presents an access token for **restricted**
connectors on the hub, and what it refuses to do with one.

## 1. Why

A connector panel can change kernel state only when the hub has signed it
(`PANEL_WRITE_GATE_DESIGN.md`): the gate requires the tree seal from a
hub-verified install and a catalog trust level of `standard` or above. The
public catalog is readable by anyone, so a panel built for one organisation
would publish that organisation's agent names and screen layout.

The hub therefore supports **restricted** connectors: signed like any other,
but listed and served only to a kernel that holds an access token covering
them and proves, on every request, that it is the kernel the token was bound
to. Nothing about seals, the JWKS or install verification changes, so the
panel write gate works unchanged for a restricted panel.

## 2. The token and the key

- An access token starts with `chubr_`. It is a separate credential class from
  a publisher token (`chub_`), which can mint seals and is never accepted here.
- It is issued by a person on the hub, for a list of connectors, and lives for
  three months. It must be bound within 24 hours of issue.
- On first use the kernel creates an Ed25519 key pair and **binds** the token
  to its public key. From then on the hub honours the token only on requests
  signed with that key. A leaked token is useless without the kernel's key.
- The key's fingerprint is the SHA-256 hex of its public key bytes. The hub
  shows it next to each token, so a person can see which kernel holds it.

## 3. Presenting a token

A request that carries the token sends three headers:

```
Authorization: Bearer chubr_…
X-Cloto-Kernel-Signature: <base64 Ed25519 signature>
X-Cloto-Kernel-Timestamp: <RFC 3339, UTC, whole seconds>
```

The signature is made with key id `cloto-kernel-access-v1` over

```
METHOD \n PATH-with-query \n TIMESTAMP \n SHA-256-hex(token)
```

with `\n NONCE` appended on renewal. The hub rejects a timestamp more than five
minutes from its clock. The nonce is signed, not only sent, so a renewal
captured in flight cannot be replayed with a fresh nonce.

**The headers go only to the hub the token was bound on**, and only while that
hub is the one this kernel is configured to use (the hub base is derived from
`CLOTO_CATALOG_URL`). A catalog entry can point a download at any host; a
token sent anywhere else would be handed to whoever runs it. A request that
carries the token never follows a redirect: the catalog fetch answers a 3xx
instead of following it, and the install engine never follows one at all.

Two requests carry it:

| Request | Who sends it |
| --- | --- |
| `GET /api/catalog` | the kernel, when it refreshes the marketplace |
| the connector download (a `raw_url` on the hub) | the install engine, which receives the headers over stdin, never argv |

## 4. Routes (operator only)

| Route | Does |
| --- | --- |
| `GET /api/hub-access` | Status: token prefix (class + four characters), token id, connectors, expiry, fingerprint, hub origin, and `stage` (`valid` / `expires_soon` / `expired`). Never the token or the key. |
| `POST /api/hub-access/token` | Bind a token (`{"token": "chubr_…"}`) and keep it. Replaces a stored one. |
| `POST /api/hub-access/renew` | Exchange the stored token for a new one. The hub revokes the old token in the same transaction. |
| `DELETE /api/hub-access/token` | Forget the token. The key stays, so the next token binds to the same fingerprint. |

These are refused to:

- **an agent token**, even when the admin key is sent alongside it. Renewing
  the kernel's hub credential is not something an agent may do for anyone;
- **the panel write relay.** `/api/hub-access` is on the gate's deny list
  (`panel_writes::DENIED_WRITE_PREFIXES`, next to `/api/modules`), so a panel
  that declares one of these routes is not usable at all.

Renewal is manual by design: the hub cannot tell a button press from a
script, so the kernel is where "only the operator" is enforced.

## 5. Storage

The key and the token live in `data_dir/hub-access/` (`kernel-access.key`,
`token.json`), written atomically with mode `0600`, in the same class as the
admin key. No API response carries either.

## 6. When the hub says no

- **Catalog.** A `401` to a token-bearing catalog request raises a notice and
  the catalog is fetched again without the token. The public catalog is still
  the public catalog; a lapsed token must not take the marketplace down.
- **Download.** A `401` fails that install and raises the same notice.
- **Unlisted installs.** The marketplace reports installs that no catalog
  entry accounts for. A connector the stored token covers is excluded: the
  public catalog never lists it, so it is restricted, not dropped.
- **Installed connectors keep working.** Their files, seal and install receipt
  are local. A panel keeps writing after its token expires; only updates stop.

## 7. Notices

From 30 days before expiry the kernel raises one notice a day (severity
`warning`; `error` once expired), settling the previous day's, until the token
is renewed or forgotten. A refused token raises one notice a day as well.
Renewing, replacing or forgetting the token settles all of them. The check runs
at boot and hourly.

## 8. Verification

Kernel unit tests cover: headers reach the bound hub and no other origin; the
signature verifies over exactly what the hub reconstructs, nonce included;
bind refuses a publisher token without calling the hub and refuses a hub that
recorded another key; renew replaces the token, refuses an expired one without
calling the hub, and keeps the token on a `401`; a catalog redirect is not
followed; a catalog `401` falls back to the public view and raises a notice;
every route refuses an agent token; the relay refuses a panel that declares the
renewal route; an expired token does not stop an installed panel's writes; the
renewal window opens at exactly 30 days. The install engine's tests cover
headers on the download, no redirect, and the `401` status reaching the kernel.
