# MGP Specification — Review Response Document

**Date:** 2026-02-28
**Spec Version:** 0.4.0-draft (post-review)
**Previous Version:** 0.3.0-draft

---

## Overview

This document records the expert review feedback received on MGP 0.3.0-draft,
the analysis of each concern, the resolution applied, and the affected sections.
It serves as a changelog rationale for the 0.3.0 → 0.4.0 revision.

---

## Concern 1: "MCP Superset" Political Vulnerability

**Original Feedback:** MGP positions itself as an "MCP superset" adding security
features. If Anthropic officially adds security features to MCP, MGP's
differentiation could be eliminated overnight. The reviewer recommended adding a
migration policy and emphasizing §16 as the core differentiator.

**Analysis:** This is a legitimate strategic risk. However, MGP's value is not
limited to security. §16 (Dynamic Tool Discovery) addresses a structural
limitation of MCP's architecture that cannot be fixed by adding features to MCP
without introducing a kernel/orchestrator concept. Security features (§3-7) are
the most likely to be absorbed by MCP; intelligence features (§16) are the least.

**Resolution:**
- Added §1.6 (Relationship to MCP & Migration Policy) with:
  - Explicit commitment to provide compatibility layers when MCP adopts equivalent functionality
  - Deprecation timeline (at least one minor version of overlap)
  - Migration Categories table showing likelihood per extension category
  - Strategic Differentiation subsection emphasizing §16 as MGP's unique value
- Updated §16.1 with a note on strategic significance and cross-reference to §1.6

**Affected Sections:** §1.6 (new), §16.1 (updated)

---

## Concern 2: Permission Request Method Naming

**Original Feedback:** `mgp/permission/request` goes Client → Server, but the
name implies the server is requesting permission from the client. The direction
is counterintuitive. Suggested renaming to `await`/`grant` to reflect roles.

**Analysis:** The method names were inherited from an early design where the
semantics were different. In the current design, the Client initiates the
permission flow (tells the server to wait, then delivers the decision). The
names "request" and "response" clash with JSON-RPC's own request/response
terminology and with the intuitive understanding that "a server requests
permission."

**Resolution:**
- Renamed `mgp/permission/request` → `mgp/permission/await`
  (Client tells Server: "await my decision")
- Renamed `mgp/permission/response` → `mgp/permission/grant`
  (Client delivers the grant/deny decision to Server)
- Updated flow diagram in §3.4 with clearer annotations
- Both methods are now Client → Server, explicitly documented as consistent
  with MCP's transport model
- Updated JSON examples and descriptions in §3.5 and §3.6

**Affected Sections:** §3.4, §3.5, §3.6

---

## Concern 3: §16 Semantic Search Embedding Dependency

**Original Feedback:** The Tool Index (§16.4) includes an `embedding` field,
implying every kernel needs an embedding model. This is a heavyweight
requirement that could discourage lightweight implementations. Suggested making
semantic search optional and clarifying Tier 4 compliance requirements.

**Analysis:** Semantic search is a quality-of-life feature, not a correctness
requirement. Keyword and category search provide sufficient tool discovery for
most use cases. Requiring embeddings would raise the barrier to Tier 4
compliance unnecessarily.

**Resolution:**
- Marked `embedding` field as OPTIONAL in §16.4 Tool Index JSON
- Added "Semantic Search is Optional" subsection with:
  - Explicit statement that keyword + category search alone is sufficient for
    `tool_discovery` extension compliance
  - Four alternative approaches for semantic search (server-side, dedicated
    service, pre-computed, none)
  - Fallback behavior specification when semantic search is requested but
    unavailable
- Updated `MGP_ADOPTION.md`:
  - Added Tier 4 compliance clarification (semantic search not required)
  - Updated implementation difficulty from `~1500行/高` to `~800-1500行/中〜高`

**Affected Sections:** §16.4 (updated), `MGP_ADOPTION.md` §2.5, §3.1

---

## Concern 4: Versioning Strategy Undefined

**Original Feedback:** §2.4 only states "major version must match, minor is
backward compatible" but says nothing about the 0.x period where breaking
changes are expected. Suggested explicit 0.x rules and 1.0 stability criteria.

**Analysis:** This is a significant gap. Without explicit 0.x rules,
implementors cannot make informed decisions about compatibility or plan their
adoption timeline.

**Resolution:**
- Added §2.5 (Versioning Policy) with three subsections:
  - **Stable Releases (1.0.0+)**: Standard semver rules
  - **Pre-1.0 Period (0.x.y)**: Minor versions MAY break, patches MUST NOT;
    implementations should attempt connection with version mismatch but log warnings
  - **1.0.0 Stability Milestone**: Four concrete criteria (two independent
    implementations, conformance test suite, 6-month stability period, mgp-validate
    tool coverage)

**Affected Sections:** §2.5 (new)

---

## Concern 5: Audit Event Transport Problem

**Original Feedback:** §6 says the kernel emits `notifications/mgp.audit` to
the Audit Server, but MCP notifications only flow between connected
client/server pairs. How does the kernel deliver notifications to the Audit
server? The transport mechanism should be explicitly documented.

**Analysis:** The answer is architecturally simple: the kernel IS the MCP
client to all servers, including the Audit server. Therefore, kernel → audit
notifications use standard Client → Server notification delivery. This was
implied by ClotoCore's architecture but never stated explicitly in the spec.

**Resolution:**
- Added "Audit Event Delivery" subsection in §6.3 with:
  - Explicit statement that the kernel acts as MCP client
  - Diagram showing Client → Server notification flow
  - Clarification that no special transport mechanism is required
- Updated `MGP_PATTERNS.md` §4.3 architecture description with the same
  clarification

**Affected Sections:** §6.3 (updated), `MGP_PATTERNS.md` §4.3 (updated)

---

## Concern 6: `creating` Status Security Risk

**Original Feedback:** §16.6's `creating` status allows agents to generate
arbitrary tool code. Combined with `auto_approve`, this creates a wide attack
surface for code injection. Suggested defaulting to disabled, making generated
tools ephemeral, and requiring `trust_level: "experimental"`.

**Analysis:** The concern is valid. Autonomous tool creation is a powerful
feature that should be opt-in with multiple safety layers, not an automatic
behavior. The original spec lacked sufficient guardrails for production use.

**Resolution:**
- `creating` status is now **DISABLED by default** (requires explicit opt-in
  via `tool_creation: { enabled: true }` in capability negotiation)
- Six mandatory safety guardrails:
  1. Opt-in required in capability negotiation
  2. Generated tools are ephemeral (session-scoped, auto-cleanup)
  3. Generated tools always receive `trust_level: "experimental"`
  4. Code Safety Framework validation mandatory at `strict` level
  5. `interactive` policy requires operator approval
  6. `TOOL_CREATED_DYNAMIC` audit event must be emitted
- New audit event type `TOOL_CREATED_DYNAMIC` added to §6.4

**Affected Sections:** §16.6 (rewritten), §6.4 (updated)

---

## Strategic Addition S1: Migration Policy

Addressed together with Concern 1. See Concern 1 resolution above.

## Strategic Addition S2: §16 as Core Differentiator

**Context:** The reviewer identified §16 (Dynamic Tool Discovery) as MGP's
most strategically important feature — the one MCP is least likely to replicate.

**Resolution:** Added strategic significance note to §16.1 with cross-reference
to §1.6. Emphasized that MCP's structural limitation (no kernel/orchestrator
layer) makes §16 unlikely to be superseded.

**Affected Sections:** §16.1 (updated)

## Strategic Addition S3: "Experience First"

**Context:** The reviewer noted that `mgp-validate` and a minimal sample
server would be more compelling than the spec document alone. "5 minutes to
MGP-compatible server" is the adoption pitch.

**Resolution:**
- Added "5分でMGP対応サーバー" emphasis to `MGP_ADOPTION.md` §5.1
- Added "最速パス" note to `MGP_ADOPTION.md` §6.2

**Affected Sections:** `MGP_ADOPTION.md` §5.1, §6.2

---

## Summary of All Changes

| Section | Change Type | Description |
|---------|-------------|-------------|
| §1.6 | New | Migration Policy and strategic differentiation |
| §2.5 | New | Versioning Policy (0.x rules, stability milestone) |
| §3.4-3.6 | Modified | Permission methods renamed (await/grant) |
| §6.3 | Modified | Explicit audit event transport documentation |
| §6.4 | Modified | Added `TOOL_CREATED_DYNAMIC` event type |
| §16.1 | Modified | Strategic differentiator emphasis |
| §16.4 | Modified | Semantic search explicitly optional |
| §16.6 | Rewritten | Safety guardrails for `creating` status |
| §9 | Modified | Version history entry for 0.4.0-draft |
| `MGP_PATTERNS.md` §4.3 | Modified | Audit transport clarification |
| `MGP_ADOPTION.md` §2.5 | Modified | Tier 4 requirements clarification |
| `MGP_ADOPTION.md` §3.1 | Modified | Implementation difficulty update |
| `MGP_ADOPTION.md` §5.1 | Modified | Experience-first emphasis |
| `MGP_ADOPTION.md` §6.2 | Modified | Fastest-path note |

---

## Post-Review: Selective Minimalism (0.5.0-draft)

**Context:** Following the expert review, a structural analysis revealed that MGP's
25 protocol primitives could be reduced to 9 without any loss of security guarantees
or MCP structural limitation breakthroughs.

**Analysis:** 16 of the 25 protocol methods are kernel-side operations (access control,
health checks, discovery, tool search). These do not require bidirectional protocol
agreement — they are invoked by operators/agents against the kernel, which is the sole
enforcement point. Converting these to standard MCP tools via `tools/call` preserves
all functionality while reducing the protocol surface area by 64%.

**Resolution — Selective Minimalism Architecture:**
- **Layer 1 (Metadata):** `_mgp` fields on existing MCP messages — 0 new methods
- **Layer 2 (Notifications):** 5 protocol notifications (audit, stream, lifecycle, callback)
- **Layer 3 (Methods):** 4 irreducible protocol methods (permission/await, permission/grant,
  callback/respond, stream/cancel) — retained because they require bidirectional agreement
- **Layer 4 (Kernel Tools):** 16 methods converted to standard MCP tools with `mgp.*`
  naming convention

**Affected Sections:**
- §1.6 (new) — Protocol Architecture diagram and rationale
- §1.7 — Migration Policy renumbered from §1.6
- §2 — Extensions table updated with Layer column
- §5 — Access Control converted to Kernel Tool Layer
- §8 — Implementation Notes rewritten for kernel tool model
- §9 — Version history entry for 0.5.0-draft
- §11 — Health/Shutdown converted to Kernel Tools, notifications retained
- §13 — Event subscription converted to Kernel Tools, callbacks retained
- §15 — Registry methods converted to Kernel Tools, config discovery retained
- §16 — All 4 methods converted to Kernel Tools, strategic content preserved
- `MGP_ADOPTION.md` — Tier descriptions and difficulty matrix updated
