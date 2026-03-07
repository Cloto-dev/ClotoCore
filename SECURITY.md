# Security Policy

## Supported Versions

| Version         | Supported |
|-----------------|-----------|
| 0.6.x (latest)  | Yes       |
| < 0.6.0         | No        |

## Reporting a Vulnerability

If you discover a security vulnerability in ClotoCore, please report it responsibly.

**Do not open a public GitHub issue for security vulnerabilities.**

### Preferred: GitHub Private Vulnerability Reporting

Use [GitHub's private vulnerability reporting](https://github.com/Cloto-dev/ClotoCore/security/advisories/new) to submit a report directly through the repository.

### Alternative: Email

Send an email to **ClotoCore@proton.me** with:

- A description of the vulnerability
- Steps to reproduce it
- The affected version(s)
- Any potential impact assessment

We will acknowledge receipt within 48 hours and provide an initial assessment within 7 days.

## Security Model

ClotoCore uses a defense-in-depth approach:

- **API authentication**: Admin endpoints require an API key (`X-API-Key` header). Keys are verified using constant-time comparison and can be invalidated system-wide. Revoked key hashes are persisted with TTL-based cleanup.
- **Rate limiting**: Per-IP rate limiting (10 req/s, burst 20) on all API endpoints via the `governor` crate.
- **MCP server isolation**: MCP servers run as separate subprocesses with independent lifecycles. Tool execution is sandboxed through the MCP protocol boundary.
- **Tool access control**: Per-agent, per-server, and per-tool access rules with 3-level priority resolution (agent > server > global). Default policy is configurable (opt-in or opt-out).
- **Command approval gate**: Agentic tool calls require human approval unless explicitly trusted. Session-scoped trust and YOLO mode are available for development workflows.
- **Permission system**: Plugin capabilities (NetworkAccess, MemoryRead, MemoryWrite) require explicit admin grants. Permission requests go through a human-in-the-loop approval flow.
- **Audit logging**: All permission grants, denials, and security-relevant events are recorded in an append-only SQLite audit log.
- **Password-protected operations**: Destructive agent operations (deletion, power toggle) support optional password verification using Argon2 hashing.
- **Secret scanning**: GitHub secret scanning and push protection are enabled on the repository.

## Disclosure Policy

- We follow coordinated disclosure practices
- Security fixes are released as patch versions when possible
- Advisories are published through GitHub Security Advisories
