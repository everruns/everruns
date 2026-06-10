# System-wide Outbound Allowlist

## Intent

An optional, host-owned global allowlist ("green list") of well-known public
resources that the egress boundary permits. It is a deployment-wide safety net
that constrains *all* outbound traffic to a curated set of trusted public
services, independently of per-agent/session `NetworkAccessList`.

Unlike `NetworkAccessList`, this is **not** end-user or agent configuration. It
is an internal, maintainer-curated list shipped with the binary. Operators turn
it on or off; they do not edit it per deployment.

## Data Model

Source of truth is an embedded TOML file, `crates/core/src/system_allowlist.toml`,
organized into named groups so it stays manageable instead of one flat list:

```toml
[groups.package_registries]
description = "Language and package manager registries (npm, crates, pypi, ...)."
allowed = ["*.npmjs.org", "*.crates.io", "*.pypi.org", ...]

[groups.ai_providers]
description = "LLM and AI provider APIs."
allowed = ["*.openai.com", "*.anthropic.com", "*.googleapis.com", ...]
```

Each group has an optional `description` and a list of `allowed` host patterns.
Patterns use the same format and matching rules as `NetworkAccessList` (see
`specs/network-access.md`):

- `example.com` — exact domain
- `*.example.com` — domain and all subdomains (apex included)
- `https://example.com/api/` — URL prefix

Current groups: `package_registries`, `source_hosting`, `container_registries`,
`ai_providers`, `cloud_providers`, `os_packages`, `developer_tools`.

`SystemAllowlist` flattens all group patterns into a single non-empty `allowed`
`NetworkAccessList`, so only URLs matching at least one pattern are permitted.
See `crates/core/src/system_allowlist.rs`.

## Enabling

Disabled by default. Controlled by a single environment variable:

```
EVERRUNS_SYSTEM_ALLOWLIST_ENABLED=true   # or 1
```

`DirectEgressService::from_env()` resolves the allowlist at construction; the
default OSS platform (`crates/server/src/platform.rs`) wires it in. When unset
or falsey, egress behavior is unchanged.

## Enforcement

The `EgressService` is the single host-owned outbound boundary (see
`specs/egress.md`). When the system allowlist is active, `DirectEgressService`
denies any request whose URL does not match the allowlist, with
`EgressError::NetworkAccessDenied`.

This check is **global**: it applies to every `EgressRequestKind` — LLM provider
calls, capabilities, integrations, system email, utility LLM, and MCP — and is
independent of the per-request `network_access`. Both the system allowlist
(if active) and the per-request `NetworkAccessList` (if present) must permit a
URL for the request to proceed.

### Operator responsibility

Because enforcement covers provider traffic too, an operator enabling the
allowlist must ensure every endpoint their deployment depends on (LLM providers,
email provider, model discovery, configured MCP servers) is covered by a group.
Self-hosted or uncommon endpoints not present in the curated groups will be
blocked while the allowlist is enabled. The curated list intentionally includes
the major AI and cloud providers so the common case works out of the box.

## Relationship to other controls

| Control | Scope | Configured by |
|---------|-------|---------------|
| System allowlist | Whole deployment, all egress kinds | Maintainers (curated), operator toggles via env |
| `NetworkAccessList` | Per harness/agent/session, agent-authored URLs | Users/agents |
| Future Egress Gateway | Network component owning outbound policy | Deployment |

The system allowlist is a precursor to the Egress Gateway's outbound allowlist
described in `specs/egress.md`: it gives a single deployment-wide allowlist
today, in-process, ahead of the remote gateway.

## Threat Model

Reinforces **TM-AGENT-018** (outbound URL filtering) with a deployment-wide
backstop that does not depend on per-agent configuration being set correctly,
and narrows the blast radius of SSRF or exfiltration attempts to a curated set
of public services.
