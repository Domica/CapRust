# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.9.x   | Yes       |
| < 0.9   | No        |

## Reporting a Vulnerability

If you discover a security vulnerability in CapRust, please report it
responsibly.

1. **Do not** open a public issue for a security vulnerability.
2. Email the maintainer at **domagoj.car@gmail.com** with:
   - A description of the vulnerability
   - Steps to reproduce
   - Potential impact
3. You will receive a response within 72 hours.
4. A fix will be released as soon as possible, and you will be credited
   in the release notes (unless you prefer to remain anonymous).

## Scope

CapRust is a local-first desktop and mobile video editor. It does not
run a server accessible from the internet. The main attack surfaces are:

- **Media files**: parsing untrusted video/audio/image files
- **Plugin importers**: third-party importer programs
- **MCP server**: JSON-RPC over stdio or loopback TCP (localhost only)
- **FFmpeg download**: verifies SHA-256 against published checksums

## Out of scope

- Vulnerabilities in dependencies (report to the upstream project)
- Issues that require physical access to the machine
- Social engineering attacks
EOF