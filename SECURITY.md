# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | Yes       |

## Reporting a Vulnerability

If you discover a security vulnerability in Exiv, please report it responsibly.

**Do not open a public GitHub issue for security vulnerabilities.**

Instead, send an email to **exiv.project@proton.me** with:

- A description of the vulnerability
- Steps to reproduce it
- The affected version(s)
- Any potential impact assessment

We will acknowledge receipt within 48 hours and provide an initial assessment within 7 days.

## Security Model

Exiv uses a defense-in-depth approach:

- **Plugin sandboxing**: Plugins run with minimal permissions by default. Elevated permissions require explicit admin approval through the human-in-the-loop system.
- **API authentication**: Admin endpoints require an API key (`X-API-Key` header). Rate limiting (10 req/s per IP, burst 20) protects against abuse.
- **Audit logging**: All permission grants, denials, and security-relevant events are recorded in an append-only SQLite audit log.
- **Network restrictions**: Plugin network access is limited to a configurable host whitelist.
- **Python bridge isolation**: Python scripts execute in a sandboxed subprocess with automatic restart on failure.
