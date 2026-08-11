# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in tuika, please report it responsibly.

**Do not open a public GitHub issue for security vulnerabilities.**

Instead, please email security issues to: **security@everruns.com**

Include:

- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Any suggested fixes (optional)

## Response Timeline

- **Acknowledgment**: Within 48 hours
- **Initial assessment**: Within 7 days
- **Resolution target**: Within 30 days for critical issues

## Scope

This security policy applies to:

- The `tuika` and `tuika-codeformatters` crates
- Official documentation and examples in this repository

## Security Model

tuika is a terminal UI library. It performs no network I/O, opens no files,
spawns no processes, and reads no credentials; its only I/O is writing bytes to
a terminal and reading terminal input events. The security-relevant surface is
therefore what it writes to the terminal and what it does with untrusted
content:

| Boundary | Consideration |
| --- | --- |
| Terminal escape sequences | Host-supplied text is rendered as cell content. Out-of-band escapes (OSC 8 hyperlinks, OSC 52 clipboard, OSC 9;4 progress, and the graphics protocols) are emitted only through tuika's own encoders, from data the host passes explicitly. A defect that let arbitrary caller text reach the terminal as an unescaped control sequence would be in scope. |
| Clipboard | OSC 52 writes are host-initiated, or runner-initiated after an explicit user drag selection over rendered cells. Terminals differ in whether they honor clipboard writes from a remote session; tuika never reads the clipboard back. |
| Graphics protocols | Image payloads are host-decoded RGBA. tuika does not decode image files, so image-parser vulnerabilities belong to the host's dependency surface, not tuika's. |
| Untrusted content | Markdown and code passed to `Markdown`/`CodeBlock` is parsed and laid out, never executed. Pathological input should degrade to slow or truncated rendering; a reproducible panic or unbounded allocation from parsed content is in scope. |

## Supported Versions

| Version | Supported |
| --- | --- |
| 0.4.x | Yes |

## Acknowledgments

We appreciate responsible disclosure and will acknowledge security researchers
who report valid vulnerabilities with permission.
