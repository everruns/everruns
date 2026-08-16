# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in Everruns, please report it
responsibly.

**Do not open a public GitHub issue for security vulnerabilities.**

Instead, please email security issues to: **security@everruns.com**

Include:
- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Any suggested fixes (optional)

You may also use [GitHub private vulnerability reporting](https://github.com/everruns/everruns/security/advisories/new).

## Response Timeline

- **Acknowledgment**: Within 48 hours
- **Initial assessment**: Within 7 days
- **Resolution target**: Within 30 days for critical issues

## Scope

This security policy applies to:
- The Everruns server, worker, and durable execution engine (`crates/`)
- The web UI (`apps/ui`)
- First-party integrations and channels (`integrations/`, Slack, A2A, MCP)
- Official SDKs, examples, and documentation

Out of scope: third-party services Everruns integrates with (LLM providers,
cloud sandboxes), self-hosted deployments that diverge from the documented
configuration, and findings that require an already-compromised host or
operator credentials.

## Security Model

Everruns is a durable agentic harness engine that runs untrusted agent and
tool code on behalf of multiple tenants. Security is a core design goal. Key
boundaries:

| Boundary | Protection |
|----------|------------|
| Tenant isolation | Org-scoped queries, `ResolvedOrg` extractor, 404 on cross-org access |
| Authentication | JWT (15 min), personal access tokens (SHA-256), OAuth, Argon2id passwords |
| Authorization | Permission policy enforced in `Command::run`, role→permission mapping |
| Encryption at rest | AES-256-GCM envelope encryption for API keys and secrets |
| Bash sandbox | Bashkit virtual interpreter, no process spawning, VFS adapter, resource limits |
| SQLite sandbox | Authorizer callback, VFS isolation, query timeouts |
| Tool execution | Registry-based validation, defensive MCP parsing, session-scoped tools |
| Network egress | SSRF protection (DNS-pinned resolve-then-check), domain allowlists |
| Resource limits | Input sizes, agent loop iteration caps, query timeouts |

Threats are tracked with stable `TM-<CATEGORY>-<NNN>` IDs. See the
[threat model](knowledge/security/threat-model.md) for the full analysis across all
categories and the [security testing process](knowledge/security/security-testing.md) for
how mitigations are verified.

### Known Limitations

Documented accepted risks and caller responsibilities are listed in the
[threat model](knowledge/security/threat-model.md) under "Accepted Risks" and "Caller
Responsibilities".

## Supported Versions

Everruns is released continuously from `main`. Security fixes target the
latest release; there is no long-term support for older tags.

## Acknowledgments

We appreciate responsible disclosure and will acknowledge security researchers
who report valid vulnerabilities (with permission).
