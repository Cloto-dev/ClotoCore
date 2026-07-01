# MCP Server Logs — Design

**Status:** Proposed
**Scope:** CScheduler Scope #12 · Goal #141
**Author:** kernel team · 2026-07-01

Per-server log streaming for the MCP server detail modal's **Log** tab
(`McpServerLogsTab`). Today the tab renders an empty "Coming soon…" state
forever. This document specifies a **hybrid** implementation that surfaces two
independent log sources through one typed event, with the **source kind visible
in the UI**.

---

## 1. Problem & current state

The Log tab is wired end-to-end on the frontend but never shows anything,
because of gaps on **both** sides. All findings below were confirmed by code
inspection on 2026-07-01.

### Backend — no per-server log events are ever emitted

- **Child `stderr` is captured but dropped.** The stdio transport's "Logger
  Task" reads every stderr line and only writes it to the kernel's own tracing
  log: `warn!("[MCP:{}] {}", cmd_display, line)` —
  `crates/core/src/managers/mcp_transport.rs:404-411`. It is never forwarded to
  the dashboard. `stdout` is consumed as the JSON-RPC transport
  (`mcp_transport.rs:381-402`), not a log sink.
- **The MCP logging capability is unhandled.** There is no handling of
  `notifications/message` or `logging/setLevel` anywhere in `crates/`.
- **No log-bearing event type.** `ClotoEventData`
  (`crates/shared/src/lib.rs:545`) has only `McpNotification { server_id,
  method, params }` (`:668-673`) and `McpCallbackRequested` (`:674-683`) as
  server-originated variants.
- **The notification forwarder whitelists MGP methods only.** In the response
  consumer (`crates/core/src/lib.rs:859-883`) only methods starting with
  `notifications/mgp.` or `notifications/cloto.` become an `McpNotification`
  event on the bus; everything else — including the standard MCP
  `notifications/message` — hits the `else` branch and is `debug!`-logged, i.e.
  dropped (`:877-883`).

### Frontend — the tab is wired but its filter can never match

`dashboard/src/components/mcp/McpServerLogsTab.tsx` subscribes to the shared SSE
event stream (`useEventStream(EVENTS_URL, …)`, `:46`) and is ready to render.
Two latent filter defects (tracked as **bug-423** and **bug-424**) mean the
predicate at `:32` can never be true for a real event:

- **bug-423** — reads `event.payload.server_id` (`:28-29`), but kernel SSE
  events carry their fields under `event.data` (see §2). `payload` is always
  `undefined`, so `serverId` is always `undefined`.
- **bug-424** — the fallback `event.type?.includes('MCP')` (`:32`) uses
  uppercase `MCP`, but the real discriminants are mixed-case `Mcp`
  (`McpNotification`, `McpCallbackRequested`). `'McpNotification'.includes('MCP')`
  is `false`.

Both are currently harmless (no log events flow) but must be fixed for this
feature; they are fixed under Task #125.

### MGP spec

MGP defines **no** dedicated logging capability (the only mention is a
transport-tradeoff note in `mgp-spec/docs/MGP_GUIDE.md:456`). MGP is a *strict
MCP superset* (`docs/MGP_SPEC.md`), so the base **MCP** logging facility
(server capability `logging`, `notifications/message`, `logging/setLevel`) is
available and is what Source B below uses. A future MGP-native logging
extension is out of scope here but not precluded.

---

## 2. Event serialization contract (why `event.data`)

`ClotoEventData` is serialized with:

```rust
#[serde(tag = "type", content = "data")]  // crates/shared/src/lib.rs:543
pub enum ClotoEventData { … }
```

So every SSE event on `/events` has the shape `{ "type": "<VariantName>",
"data": { …fields… }, "timestamp": … }`. Consumers therefore read fields from
`event.data.*` and switch on `event.type === '<VariantName>'` — e.g.
`useVrmAvatar.ts:21` (`event.data.agent_id`), `AgentConsole.tsx:360`
(`event.data.agent_id`), `VrmViewerPage.tsx:143` (`event.type ===
'McpNotification'`). The Log tab must follow the same contract (this is exactly
what bug-423 / bug-424 get wrong).

---

## 3. Goals / non-goals

**Goals**

1. Surface real per-server logs in the Log tab, filtered to the open server.
2. Cover **all** MCP servers via `stderr` (Source A) and, additionally,
   **structured** logs (level + logger) from servers that implement the MCP
   logging capability (Source B).
3. Make the **source kind** (`stderr` vs MCP logging) visible in the UI — an
   explicit requirement.
4. Fix the two frontend filter defects (bug-423, bug-424).

**Non-goals**

- Persisting logs across kernel restarts (the tab is a live tail; ring-buffered
  in-memory on the client, last 200 lines — matches current `slice(-199)`).
- A global/aggregate log viewer (this is per-server).
- An MGP-native logging protocol extension.
- `logging/setLevel` UI controls beyond an optional kernel-set default (see §6).

---

## 4. Design overview

One new typed event, `ClotoEventData::McpServerLog`, is the single vehicle for
both sources. Both routes tag the originating `server_id` and set a `source`
discriminator; the event flows through the existing system event bus onto the
SSE `/events` stream that `McpServerLogsTab` already consumes.

```
Source A (stderr):   child stderr line ──▶ StdioTransport stderr channel
                       ──▶ owning MCP client (tags server_id)
                       ──▶ McpServerLog{ source: Stderr, level: None } ──▶ bus ──▶ SSE
Source B (MCP log):  server `notifications/message` ──▶ response consumer arm
                       ──▶ McpServerLog{ source: McpLogging, level, logger } ──▶ bus ──▶ SSE
Frontend:            SSE ──▶ McpServerLogsTab (type === 'McpServerLog',
                       data.server_id === server.id) ──▶ render with source + level badges
```

---

## 5. The `McpServerLog` event

Add to `ClotoEventData` (`crates/shared/src/lib.rs`, adjacent to
`McpNotification` / `McpCallbackRequested`):

```rust
/// A log line from an MCP/MGP server, surfaced to the dashboard Log tab.
/// Two sources are unified here and distinguished by `source`.
McpServerLog {
    server_id: String,
    /// Where the line came from — shown as a badge in the UI.
    source: McpLogSource,
    /// RFC 5424 severity. Present for MCP-logging lines; None for raw stderr.
    #[serde(skip_serializing_if = "Option::is_none")]
    level: Option<McpLogLevel>,
    /// Optional logger/category name (MCP logging `logger` field).
    #[serde(skip_serializing_if = "Option::is_none")]
    logger: Option<String>,
    message: String,
    /// RFC3339; kernel receive time (stderr) or notification time (MCP logging).
    timestamp: String,
},
```

New supporting enums (in `crates/shared/src/lib.rs`):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]   // "stderr" | "mcp_logging"
pub enum McpLogSource { Stderr, McpLogging }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]    // RFC 5424 / MCP
pub enum McpLogLevel {
    Debug, Info, Notice, Warning, Error, Critical, Alert, Emergency,
}
```

Serialized wire form (adjacently tagged, per §2):

```json
{ "type": "McpServerLog",
  "data": { "server_id": "mind.local", "source": "stderr",
            "message": "…", "timestamp": "2026-07-01T07:12:00Z" },
  "timestamp": "…" }
```

**Fixed point:** the variant name is `McpServerLog`; `source` values are
`stderr` / `mcp_logging`; fields live under `event.data` per the existing serde
contract.

---

## 6. Source A — stderr forwarding (Task #123)

**Capture point:** the Logger Task at `mcp_transport.rs:404-411`. Today it only
`warn!`s each line. Change it to also forward each line out of the transport.

**Plumbing:** the transport does not itself know the logical `server_id` (it
holds `cmd_display` only); the owning MCP client does. Mirror the existing
notification flow (stdout → `res_tx` → parsed in the client, which already tags
`server_id` before reaching the consumer at `lib.rs:868`):

1. `StdioTransport::spawn` gains an optional `stderr_tx: mpsc::Sender<String>`
   (raw lines). The Logger Task sends each line on it (and keeps the existing
   `warn!` for release-log visibility — decision below).
2. The owning client attaches `server_id` and emits
   `McpServerLog { source: Stderr, level: None, logger: None, message: line,
   timestamp: now }` onto the system event bus (same `notif_event_tx`-style
   path used for `McpNotification`).

`stdout` (the JSON-RPC channel) is untouched — no risk of polluting the RPC
stream.

**Decisions**

- **Keep the existing `warn!` local tracing** in addition to forwarding (dual
  sink). Rationale: release-log diagnostics and post-mortem `journalctl`-style
  triage should not depend on a dashboard being open. Low volume.
- **`level: None` for stderr** (no reliable structure). The UI shows the
  `[stderr]` source badge and no level badge. A heuristic parser (leading
  `[ERROR]`/`WARN`/…) is explicitly deferred — do not guess levels in v1.

---

## 7. Source B — MCP logging capability (Task #124)

MCP models logging as a **server** capability: a server that advertises
`capabilities.logging` sends `notifications/message` with `{ level, logger?,
data }`; the client may send `logging/setLevel { level }` to set the minimum
severity. **The client does not declare a `logging` capability** — so, contrary
to the initial investigation note, no `ClientCapabilities` change is required to
*receive* logs. (Verify against the MCP spec revision ClotoCore targets before
implementing; adjust if that revision differs.)

**Inbound handling:** add an arm to the response consumer whitelist
(`crates/core/src/lib.rs:859-883`) for `notif.method == "notifications/message"`
that maps it to `McpServerLog { source: McpLogging, level: params.level,
logger: params.logger, message: stringify(params.data), timestamp: now }`. The
existing `notifications/mgp.*` / `notifications/cloto.*` arm is unchanged (a
regression test pins that they still forward). Everything else keeps hitting the
drop branch.

**setLevel (v1, kernel-set default — approved):** to make servers that gate
emission on a level actually emit, the kernel sends `logging/setLevel` once
after `initialize`, only to servers that advertised `capabilities.logging`,
with the config-driven default **`info`**. This ships in **v1**. There is no
per-server UI control in v1 — a single kernel/config default applies to all
logging-capable servers. The config key SHOULD allow overriding the default
(`info`) globally.

---

## 8. Frontend (Task #125)

`McpServerLogsTab.tsx`:

1. **Fix bug-423** — read `event.data.server_id` (not `event.payload`).
2. **Fix bug-424** — match `event.type === 'McpServerLog'` (and, if desired,
   keep `McpNotification` for MGP notifications) instead of
   `includes('MCP')`.
3. **Source badge** — render `data.source` as a colored badge: `[stderr]`
   (neutral) vs `[MCP]` (brand) — the required source distinction.
4. **Level badge** — when `data.level` is present (MCP logging), render a
   severity-colored badge (`debug…emergency`); stderr shows none.
5. Extend the local `LogEntry` type with `source` / `level` / `logger`; update
   the wire type (`StrictSystemEvent`) usage accordingly.
6. Update the `logs.waiting` copy (English canonical + external language-pack
   key) away from "Coming soon…" to a live empty state (e.g. "No log output
   yet.").

UI rules (`dashboard`): min text `text-[9px]`, badges follow existing
color/`bg-glass` conventions; no new Tailwind classes without regenerating
`compiled-tailwind.css`.

---

## 9. Testing

- **Backend A:** unit/integration — a stderr line becomes an `McpServerLog{
  source: Stderr }` tagged with the right `server_id`; `stdout` RPC framing is
  unaffected.
- **Backend B:** `notifications/message` → `McpServerLog{ source: McpLogging,
  level, logger }`; the `mgp.*` / `cloto.*` forwarding does **not** regress;
  capability/`setLevel` handshake (if shipped) is exercised.
- **Frontend:** filter matches a synthesized `McpServerLog` for the open
  server and ignores other servers'; source/level badges render; bug-423 /
  bug-424 regression (payload-path and casing) covered.
- **Live:** against a running kernel — a server that logs to stderr shows
  `[stderr]` lines; a logging-capable server shows `[MCP]` lines with levels.

---

## 10. Fixed points

1. Single event type `McpServerLog`; `source ∈ { stderr, mcp_logging }`; fields
   under `event.data` (adjacent-tag serde contract, §2).
2. `stdout` stays the pure JSON-RPC channel; stderr forwarding is a separate
   channel (§6).
3. The `notifications/mgp.*` / `notifications/cloto.*` forwarding is preserved;
   `notifications/message` is added as a new arm, not a replacement (§7).
4. stderr lines carry `level: None` in v1 (no level guessing).
5. Frontend reads `event.data` and matches `event.type` exactly (§8).
6. `logging/setLevel` ships in v1 with default `info`, sent only to servers
   advertising `capabilities.logging` (§7).
7. stderr uses a dual sink: forward to the bus **and** keep the existing
   `warn!` local tracing (§6).

---

## 11. Decisions (resolved 2026-07-01)

- **D1 — resolved: setLevel ships in v1**, kernel sends `logging/setLevel` once
  after `initialize` with default `info` (config-overridable). §7.
- **D2 — resolved:** the client does not declare a `logging` capability
  (logging is a server capability); it handles inbound `notifications/message`
  and sends `logging/setLevel`. Confirm the exact field names against the
  targeted MCP spec revision at implementation time. §7.
- **D3 — resolved: keep the dual sink** (forward + `warn!`) for stderr. §6.

---

## 12. Task mapping (Goal #141)

| Task | This doc |
| --- | --- |
| #122 [design] | this document |
| #123 [backend-A] | §6 |
| #124 [backend-B] | §7 |
| #125 [frontend] | §8 (fixes bug-423 / bug-424) |
