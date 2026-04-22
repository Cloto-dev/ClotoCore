# MGP — Multi-Agent Gateway Protocol

**Version:** 0.6.0-draft
**Status:** Draft
**Authors:** ClotoCore Project
**Date:** 2026-03-06

The canonical MGP specification is maintained in the
[cloto-mcp-servers](https://github.com/Cloto-dev/cloto-mcp-servers)
repository, alongside the server implementations and the detailed
specification documents. This file is a pointer — see the canonical
documents below.

## Canonical Documents

| File | Sections | Content |
|------|----------|---------|
| [MGP_SPEC.md](https://github.com/Cloto-dev/cloto-mcp-servers/blob/main/docs/MGP_SPEC.md) | §1 | Overview, Architecture, Migration Policy |
| [MGP_SECURITY.md](https://github.com/Cloto-dev/cloto-mcp-servers/blob/main/docs/MGP_SECURITY.md) | §2-§7 | Capability Negotiation, Permissions, Tool Security, Access Control, Audit, Code Safety |
| [MGP_COMMUNICATION.md](https://github.com/Cloto-dev/cloto-mcp-servers/blob/main/docs/MGP_COMMUNICATION.md) | §11-§14 | Lifecycle, Streaming, Bidirectional Communication, Errors |
| [MGP_DISCOVERY.md](https://github.com/Cloto-dev/cloto-mcp-servers/blob/main/docs/MGP_DISCOVERY.md) | §15-§16 | Discovery, Dynamic Tool Discovery |
| [MGP_GUIDE.md](https://github.com/Cloto-dev/cloto-mcp-servers/blob/main/docs/MGP_GUIDE.md) | §17-§20 | Implementation, History, Patterns |
| [MGP_ISOLATION_DESIGN.md](https://github.com/Cloto-dev/cloto-mcp-servers/blob/main/docs/MGP_ISOLATION_DESIGN.md) | (§8-§10 reserved) | OS-Level Isolation |

## ClotoCore as the Reference Implementation

ClotoCore is the reference implementation of MGP. ClotoCore-specific
kernel tool extensions that are not part of the MGP specification
(e.g., `create_mcp_server`, `ask_agent`, `gui.map`, `gui.read`) are
documented in:

- [MCP_PLUGIN_ARCHITECTURE.md §3.2](./MCP_PLUGIN_ARCHITECTURE.md#32-clotocore-specific-extensions-custom-methods)

For the mapping of MGP specification sections to ClotoCore source-code
locations, see
[MGP_GUIDE.md §17.3](https://github.com/Cloto-dev/cloto-mcp-servers/blob/main/docs/MGP_GUIDE.md#173-relationship-to-clotocore).
