# MGP — Moved to mgp-spec

The canonical MGP specification has been split out to its own repository (MIT, independent):
**https://github.com/Cloto-dev/mgp-spec**

This file remains as a stub to keep `Mandatory Reads` paths in CLAUDE.md working and to redirect any in-tree references.

## Canonical Documents (mgp-spec)

| File | Sections | Content |
|------|----------|---------|
| [docs/MGP_SPEC.md](https://github.com/Cloto-dev/mgp-spec/blob/main/docs/MGP_SPEC.md) | §1 | Overview, Architecture, Migration Policy |
| [docs/MGP_SECURITY.md](https://github.com/Cloto-dev/mgp-spec/blob/main/docs/MGP_SECURITY.md) | §2-§7 | Capability Negotiation, Permissions, Tool Security, Access Control, Audit, Code Safety |
| [docs/MGP_COMMUNICATION.md](https://github.com/Cloto-dev/mgp-spec/blob/main/docs/MGP_COMMUNICATION.md) | §11-§14 | Lifecycle, Streaming, Bidirectional Communication, Errors |
| [docs/MGP_DISCOVERY.md](https://github.com/Cloto-dev/mgp-spec/blob/main/docs/MGP_DISCOVERY.md) | §15-§16 | Discovery, Dynamic Tool Discovery |
| [docs/MGP_GUIDE.md](https://github.com/Cloto-dev/mgp-spec/blob/main/docs/MGP_GUIDE.md) | §17-§20 | Implementation, History, Patterns |
| [docs/MGP_ISOLATION_DESIGN.md](https://github.com/Cloto-dev/mgp-spec/blob/main/docs/MGP_ISOLATION_DESIGN.md) | (§8-§10 reserved) | OS-Level Isolation |

## Migration

- Migration date: **2026-05-07**
- mgp-spec head: **v0.6.1-draft** (sanity-aligned with cloto-mcp-servers/docs/MGP_SPEC.md history through 2026-04-22)
- License: MIT (independent from ClotoCore's BSL → MIT 2028)

## ClotoCore as the Reference Implementation

ClotoCore is the reference implementation of MGP. ClotoCore-specific kernel tool extensions that are not part of the MGP specification (e.g., `create_mcp_server`, `gui.map`, `gui.read`) are documented in:

- [MCP_PLUGIN_ARCHITECTURE.md §3.2](./MCP_PLUGIN_ARCHITECTURE.md#32-clotocore-specific-extensions-custom-methods)

For the mapping of MGP specification sections to ClotoCore source-code locations, see [MGP_GUIDE.md §17.3](https://github.com/Cloto-dev/mgp-spec/blob/main/docs/MGP_GUIDE.md#173-relationship-to-clotocore).
