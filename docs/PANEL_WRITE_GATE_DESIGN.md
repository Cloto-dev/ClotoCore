# Panel Write Gate — Letting Connector Panels Change Kernel State

Status: **Accepted**, implementing (§4.1 revised during implementation)
Scope: ClotoCore kernel (`crates/core`) and dashboard (`dashboard/src`)

## 1. Problem

A connector can ship a UI panel. Installing the connector adds the panel to
the dashboard, and uninstalling it removes the panel again
(`crates/core/src/handlers/modules.rs`, `discover_connector_panels`). Today a
panel can only read. The host proxies `GET` and nothing else
(`dashboard/src/lib/moduleBridge.ts`, `PROXYABLE_METHODS`). The comment on that
constant sets the condition for lifting the limit: "widen this only alongside
a decision about how that is surfaced".

That limit now gets in the way of a real use case. One use case is an
operations console for a small organisation of agents, shipped as a connector.
It shows the organisation and what each department is doing. It also lets the
operator talk to a manager agent from the same screen. Reading the
organisation fits in the current model, but talking to an agent is a write:
it creates a conversation and sends messages that the agent answers
(`POST /api/chat/{agent_id}/conversations`, `POST /api/chat/{agent_id}/send`).

This document makes the decision that the comment asks for. It covers which
panels may write, what they may write, how the operator sees it, and where the
rule is enforced.

## 2. What exists today

| Fact | Where |
| --- | --- |
| A panel runs in a frame with `allow-scripts` only, so it has an opaque origin, no cookie, no admin key and no storage | `dashboard/src/pages/ModulePage.tsx` |
| The host decides each request with `decideModuleCall`, a pure function: `GET` only, the path must be under `/api/`, and the manifest's `requires` must declare it (exactly, or as a one-segment `/*` wildcard) | `dashboard/src/lib/moduleBridge.ts` |
| That decision runs only in the browser. The kernel has no idea that a request came from a panel | same |
| Panels come from two places: directories placed by hand under `<data_dir>/modules/`, and panels declared by installed connectors | `crates/core/src/handlers/modules.rs` |
| Trust levels are ordered `Untrusted < Experimental < Standard < Core` (re-exported from `mgp-seal`) | `crates/core/src/managers/mcp_mgp.rs` |
| When a server starts, the kernel computes an **effective** trust level. If no seal was verified, a declared level above `Untrusted` is forced down to `Untrusted` | `crates/core/src/managers/mcp.rs`, the block that computes `effective_trust_level` |
| The effective level feeds the isolation profile and is then **dropped**. It is not stored anywhere else | same |
| `NegotiatedMgp.trust_level`, which the server handle stores, comes from the MGP handshake: config first, then the server's own declaration, then `Untrusted`. **The seal downgrade is never applied to it** | `crates/core/src/managers/mcp_mgp.rs`, `negotiate` |
| HTTP-transport servers skip seal verification, and so does `CLOTO_ALLOW_UNSIGNED=true`. In both cases the declared level stays in place | `crates/core/src/managers/mcp.rs` |
| The kernel keeps a hash-chained audit log | `crates/core/src/db/audit.rs`, `write_audit_log` |
| A connector that ships only panels (`ui_module`) is never started and gets no `mcp_servers` row. Its install verifies the tree against the hub and mints a local tree seal, then keeps neither the seal nor the trust level | `crates/core/src/handlers/marketplace.rs`, `finish_static_install` |
| The in-kernel build path (monorepo tarball, git) mints a local seal only after verifying an entry point against the hub. A connector with no server has no entry point, so its tree is never sealed on that path | `crates/core/src/handlers/marketplace.rs`, `local_seal_for_install` |

The fourth-from-last and third-from-last rows matter most for this design. The
only trust value that lives past startup is the one that has not been checked
against a seal. A gate that read it would let a connector unlock writes by
declaring `core` in its own handshake.

## 3. Decisions

1. **Threshold: effective trust level `Standard` or above, and the seal was
   verified.** "Verified" is part of the rule on purpose, not just an effect
   of the downgrade. Two start paths leave the declared level in place without
   checking a seal: HTTP transport and `CLOTO_ALLOW_UNSIGNED`. Stating it
   separately stops either path from qualifying. `Core` alone would restrict
   writes to connectors built into the kernel, and the motivating panel
   would not be one of those.
2. **Surfacing: the operator consents once, per panel, to a list of writes.**
   After that, each write is recorded in the audit log, and the panel shows
   that it can write. The consent lapses when the declared list or the
   connector version changes. The operator is not asked to confirm each
   write. A chat message is a write, and a confirmation on every message would
   make the panel unusable without making anything safer.
3. **Initial scope: creating a conversation and sending a message to one named
   agent.** Control actions (pause, resume, run now, decide a proposal) come
   later, under the same gate.

   *Revised 2026-09-19.* The first version named `POST /api/chat/{agent_id}/messages`
   as the send. That route only stores a message; the agent never answers it.
   What makes an agent answer is `POST /api/chat`, and that route takes the
   target and the sender from the body, so an exact path pins neither. The
   kernel therefore gained `POST /api/chat/{agent_id}/send`: the path names the
   agent, the conversation must belong to it, and the sender is the
   conversation's owner, all decided by the kernel. It also gained
   `GET /api/chat/{agent_id}/conversations/{conversation_id}`, because the
   existing read narrows to a conversation with `?conversation_id=`, and a
   declaration cannot carry a query string (`moduleBridge.ts`,
   `matchesSegmentWildcard`). A panel reads its thread with the one-segment
   wildcard `GET /api/chat/{agent_id}/conversations/*`.

## 4. Design

### 4.1 Eligibility (kernel)

*Revised during implementation. The first draft recorded eligibility on the
server handle when the server started. That cannot work for the connector this
gate exists for: a connector that ships only panels (`connector_type:
"ui_module"`) is never started and gets no `mcp_servers` row, so there is no
handle and no start-time check. See the last rows of §2.*

A marketplace install records a **receipt** for the tree it placed, for both
kinds of connector, in a new table:

```sql
CREATE TABLE connector_install_receipts (
    connector_dir TEXT PRIMARY KEY,  -- directory under the servers root
    trust_level   TEXT NOT NULL,     -- the catalog's trust level at install
    seal          TEXT,              -- the local tree seal; NULL when unsealed
    version       TEXT NOT NULL DEFAULT '',
    installed_at  TEXT NOT NULL
);
```

Uninstalling removes the receipt along with the files.

A panel is **write-eligible** only if all of these hold, checked by the kernel
on every write:

- It was declared by a connector installed from the marketplace. A directory
  placed by hand under `modules/` has no receipt, so it can never be eligible.
  A placed module that declares `writes` is rejected from the listing.
- The receipt's trust level is `standard` or above.
- The receipt holds a tree seal (`tree-sha256:`). An entry-point seal does
  not qualify, because it does not cover the panel's files.
- That seal **still verifies** against the installed tree now.
- The panel is served from inside that tree. The tree seal neither follows nor
  hashes symlinks, but manifest discovery follows one at the fixed
  `servers/<id>/` path. Without this check, a sealed tree could hold a link
  that serves a panel from files the seal never covered.

Because the seal is verified on every write instead of once at start, a tree
changed after install is refused on the next write. Neither
`NegotiatedMgp.trust_level` nor the start-time effective level is read (see
§2).

### 4.2 Declaration (connector manifest)

Panels get a new field, `writes`, separate from `requires`:

```json
{
  "ui": {
    "panels": [{
      "id": "console",
      "name": "Operations Console",
      "requires": ["GET /api/published/*", "GET /api/chat/agent.manager/conversations/*"],
      "writes": [
        "POST /api/chat/agent.manager/conversations",
        "POST /api/chat/agent.manager/send"
      ]
    }]
  }
}
```

The kernel validates `writes` when it discovers panels. If any entry breaks a
rule, the whole panel is listed with an `error`, the same way a malformed
manifest is today. Such a panel is never listed as usable with the bad entries
quietly dropped. The rules are:

- The method is `POST` or `PATCH`. `DELETE` and `PUT` are not accepted in
  this version.
- The path is under `/api/` and exact. There are no wildcards. The kernel's
  chat routes include `delete_all_conversations` and
  `archive_all_conversations`, and a writable wildcard over a route family
  would cover whatever gets added to it later.
- Exact paths also pin the target agent. A panel that declares one agent's
  message route cannot drive another agent.

Keeping `writes` separate from `requires` means a reader of the manifest can
see the write surface without parsing methods out of a mixed list.

### 4.3 Consent (kernel, new table)

```sql
CREATE TABLE panel_write_consents (
    panel_id          TEXT PRIMARY KEY,
    writes_digest     TEXT NOT NULL,  -- SHA-256 of the canonicalised `writes` list
    connector_version TEXT NOT NULL,
    granted_at        TEXT NOT NULL,
    granted_by        TEXT NOT NULL   -- operator principal
);
```

A consent is valid only if its digest and version match the panel as it is
discovered now. Any change voids it, and the panel falls back to read-only
until the operator consents again. Revoking a consent deletes the row.

This needs a schema migration, so the implementation PR is one the owner
merges (see the release rules).

### 4.4 Enforcement point: a kernel relay route

A write does not go from the host to the target route directly. It goes
through a new route:

```
POST /api/modules/{id}/write
{ "method": "POST", "path": "/api/chat/agent.manager/send", "body": { ... } }
```

The handler runs these checks in order. Every refusal returns 403 with a reason
the panel can show:

1. The panel exists and is write-eligible (§4.1).
2. `method` and `path` exactly match an entry in the panel's `writes`.
3. A valid consent exists (§4.3).
4. The body is JSON and at most 64 KiB.
5. The panel is under its rate cap (fixed at 30 writes per minute per panel;
   see §7).

Three routes support the dashboard. Each takes `{id}` as the panel id:

| Route | Purpose |
| --- | --- |
| `GET /api/modules/{id}/write-access` | Whether the panel is eligible (and why not), what it declares, and whether a consent exists and still holds |
| `PUT /api/modules/{id}/write-consent` / `DELETE` | Give or revoke consent. `PUT` is refused for an ineligible panel |
| `GET /api/modules/write-consents` | Every recorded consent, for the settings list |

If all checks pass, the handler dispatches the inner request to the kernel's
own router in-process. It carries the caller's own credential headers over,
so the target route applies its normal authentication, and the relay never
grants more than the operator already has. The router is kept in a
`OnceLock` that is set after the router is built.

Why the kernel and not the browser: the browser check (`decideModuleCall`)
stays for reads, and for writes it keeps as a fast first filter. But the
rule has to hold even if the dashboard has a bug. The record also has to be
written by the component that performed the write. A record written by the
browser proves only that the browser meant to write.

The host bridge changes in one place. `decideModuleCall` accepts
`POST`/`PATCH` only when they are declared in `writes`, and it sends them to
`/api/modules/{id}/write`, never to the target. `PROXYABLE_METHODS` stays
`GET` for direct proxying.

### 4.5 Audit

Every relayed write writes one audit row:

| Field | Value |
| --- | --- |
| `event_type` | `PANEL_WRITE` on success, `PANEL_WRITE_DENIED` on refusal |
| `actor_id` | operator principal |
| `target_id` | panel id |
| `result` / `reason` | the target's status, or which check refused the write |
| `metadata` | `{method, path, effective_trust_level, seal_verified, writes_digest}` |

The body is not recorded, because a chat message can hold anything the
operator typed.

### 4.6 What the operator sees

- Panel header: a "Can write" badge when a consent is active. Opening the
  badge shows the declared write list.
- On first use of an eligible panel that has no consent: a consent sheet that
  lists each declared write in plain words ("Send messages to agent.manager").
  If the panel is not eligible, the sheet says why (for example, "The seal was
  not verified") and there is nothing to accept.
- Settings, Security: the active panel consents, each with Revoke.

## 5. Threats and what stops them

| Threat | Stopped by |
| --- | --- |
| A panel from an unsealed or self-promoted connector writes | §4.1: the effective level plus seal verification, checked by the kernel on each write |
| A connector declares `core` in its handshake | §4.1 never reads the handshake level |
| A panel writes to a route it did not declare, or to a whole route family | §4.2: exact declarations only, checked again in §4.4 |
| An update quietly widens the write list | §4.3: the digest and version void the consent |
| A dashboard bug skips the browser check | §4.4: the kernel repeats every check |
| The relay grants more than the operator holds | §4.4: the caller's own credential is carried to the target |
| A panel floods an agent with messages | §4.4: per-panel rate cap |
| A write happens that the operator cannot see afterwards | §4.5: every write and refusal is audited |

**Not covered:** a panel that is eligible and has consent can still send
messages the operator did not type. The operator consented to "this panel may
send messages to this agent", and a panel that abuses that is a trusted
connector misbehaving. The mitigations are the audit trail and revocation, not
prevention. This limit is why the initial scope (§3.3) stops at chat.

## 6. Verification

Each rule gets a test, and the implementation PR records that each test has
detection power. Delete or weaken the check, see that exact test go red, then
restore it and see the test go green. The tests are in
`crates/core/src/handlers/panel_writes.rs`. They install a real tree, mint a
real tree seal over it and record a receipt, so every check runs against the
same verification code a live install uses.

- Eligibility: a sealed `standard` or `core` connector is accepted.
  `experimental`, `untrusted`, no seal, an entry-point seal, no receipt, a
  tree changed after install, a hand-placed module, and a panel served through
  a link out of the sealed tree are each refused.
- The trust level is the receipt's. A connector whose own files claim `core`
  while its receipt says `experimental` is refused.
- Declarations: `DELETE`, `PUT`, `GET`, lower-case methods, wildcards, queries,
  `.`/`..`/percent-encoded segments, empty segments, non-`/api/` paths, and
  paths into the module routes are each refused. A panel that declares any of
  them is not listed as usable.
- Consent: refused for an ineligible panel. A changed declaration list and a
  changed version each void it, and revoking it stops writes.
- Relay: an admitted write reaches the target carrying the caller's own
  headers. An undeclared route, a missing consent, an oversized body, a flood
  past the cap, and a tree changed after consent are each refused before the
  target is reached.
- Audit: one row per write and one per refusal, and no row contains the body.
- Receipts: an install through the install engine records a tree seal that
  verifies (`crates/core/tests/marketplace_install_test.rs`). The in-kernel
  build path records the tree as unsealed, and uninstalling removes the
  receipt.
- Route registration: a handler test cannot show that the routes are mounted,
  because the router is built inline at boot. A structural test asserts the
  registration lines in `crates/core/src/lib.rs`.
- A live kernel: `scripts/opverify/catalog/modules.py` drives every gate route
  against an unknown panel and expects a refusal.

## 7. Open questions

- **Rate cap value.** 30 per minute per panel is a starting number, not a
  measured one. Adjust it after real use.
- **Routes with dynamic segments.** Renaming a conversation is
  `PATCH /api/chat/{agent_id}/conversations/{conversation_id}`, which exact
  declarations cannot express. It is out of the initial scope. If it is needed,
  the option to evaluate is a declared placeholder for ids the same panel
  created, not a general wildcard.
- **Streaming replies.** The first version reads replies by polling the
  declared `GET` conversation route. Streaming is a separate decision.
