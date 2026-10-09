---
type: Specification
title: "System-wide Outbound Allowlist"
description: "System-wide outbound allowlist (\"green list\")."
tags:
  - everruns
  - operations
---
# System-wide Outbound Allowlist

## Intent

An optional, host-owned global allowlist ("green list") of well-known public
resources that the egress boundary permits for tenant/agent-directed outbound
traffic. It is a deployment-wide safety net that constrains capability, MCP,
integration, and generic runtime HTTP traffic to a curated set of trusted public
services, independently of per-agent/session `NetworkAccessList`.

Unlike `NetworkAccessList`, this is **not** end-user or agent configuration. It
is an internal, maintainer-curated list shipped with the binary. Operators turn
it on or off; they do not edit it per deployment.

## Data Model

Source of truth is an embedded TOML file, `crates/contracts/src/runtime/system_allowlist.toml`,
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
`knowledge/operations/network-access.md`):

- `example.com`, exact domain
- `*.example.com`, domain and all subdomains (apex included)
- `https://example.com/api/`, URL prefix

Current groups: `package_registries`, `source_hosting`, `container_registries`,
`ai_providers`, `cloud_providers`, `os_packages`, `developer_tools`,
`agent_services`, `mcp_services`. `agent_services` holds services agents sign up with on
their own (auth.md, AgentID); `mcp_services` holds hosted MCP servers agents connect to over
OAuth (Visti, Stend). Each entry in both is reviewed because the agent can send data there.

`SystemAllowlist` flattens all group patterns into a single non-empty `allowed`
`NetworkAccessList`, so only URLs matching at least one pattern are permitted.
An allowlist with no patterns (empty/misconfigured TOML) **fails closed**: it
denies every URL rather than allowing all, via a sentinel pattern, since an
empty `NetworkAccessList.allowed` otherwise means "no restriction". See
`crates/contracts/src/runtime/system_allowlist.rs`.

## Modes

The allowlist is one part of the deployment's system egress policy
(`SystemEgressPolicy` in `crates/contracts/src/runtime/system_allowlist.rs`),
selected by one environment variable:

```
EVERRUNS_EGRESS_POLICY=open | curated-writes | curated-all
```

| Mode | Reads (GET/HEAD, no body) | Writes (any other request, plus all MCP and integration traffic) |
|---|---|---|
| `open` (default) | any public host | any public host |
| `curated-writes` | any public host not on the deny list | deny list, then allowlist |
| `curated-all` | deny list, then allowlist | deny list, then allowlist |

When `EVERRUNS_EGRESS_POLICY` is unset, the legacy
`EVERRUNS_SYSTEM_ALLOWLIST_ENABLED=true` (or `1`) selects `curated-all`. An
unrecognized `EVERRUNS_EGRESS_POLICY` value fails closed to `curated-all`.

Why reads are open in `curated-writes`: the set of pages an agent usefully
reads is unbounded, so an allowlist for reads is always wrong and became the
main source of friction on hosted. The allowlist's real job on an open-signup
deployment is stopping tenants from pushing data to, or relaying through,
arbitrary hosts, which only requests that carry data can do. A read can still
carry data in its URL, so open reads are bounded:

- **Deny list.** `crates/contracts/src/runtime/system_denylist.toml` names hosts
  whose purpose is to receive data (request bins, out-of-band testing domains,
  public tunnels). It applies in both curated modes, before the allowlist, and
  nothing overrides it.
- **Domain reputation.** With `EVERRUNS_EGRESS_REPUTATION=cloudflare-security`,
  the boundary asks Cloudflare's malware-blocking resolver
  (`security.cloudflare-dns.com`, the 1.1.1.2 service, over DNS-over-HTTPS)
  about a host before its first open read, and refuses hosts it sinkholes.
  Only the hostname leaves the deployment; verdicts are cached for an hour; a
  lookup failure fails open, because reputation narrows open reads rather than
  being what makes them safe. Off by default. Decision: a free filtering
  resolver over a paid URL-reputation API (Google Web Risk, Cloudflare
  categories), which can be added behind the same check if abuse warrants it;
  an LLM classifier was rejected because the URL it would judge is attacker
  controlled and every read would pay a model call.
- **URL length.** An open read's URL is capped at 2048 characters.
- **Hostnames only.** An open read to an IP literal is refused, since it would
  bypass any domain-based reputation.
- **Per-org rate limit.** `DirectEgressService` admits at most
  `EVERRUNS_EGRESS_OPEN_READS_PER_MINUTE` (default 120) open reads per org per
  minute per process; `0` disables open reads. Allowlisted requests are not
  metered. The browser path does not go through this meter.
- **Audit log.** Every request is logged with its policy outcome
  (`knowledge/operations/egress.md`).

`DirectEgressService::for_runtime_traffic_from_env()` resolves the policy once
per process. Host-owned system transports do not construct or call
`EgressService`, so they do not read this setting.

The env var is read by every process that builds an egress service, so it
applies uniformly across the **control plane** and **workers**:

- `crates/server/src/platform.rs`, control-plane / in-process platform.
- `crates/worker/src/platform.rs`, distributed worker platform.
- `crates/server/src/domains/mcp_servers/service/mod.rs`, MCP server egress.

Each runtime/agent egress surface must construct egress via
`DirectEgressService::for_runtime_traffic_from_env()` (not `::default()`) so the
setting is honored everywhere. The list contents are *not* env-configurable,
they are the curated embedded TOML; only the mode is environmental.

## Enforcement

The `EgressService` is the tenant/agent runtime outbound boundary (see
`knowledge/operations/egress.md`). When a curated mode is active, `DirectEgressService`
denies tenant/agent-directed requests the policy refuses, with
`EgressError::NetworkAccessDenied`.

This check applies to `capability`, `integration`, `mcp`, and generic `other`
requests, independent of the per-request `network_access`. Both the system
allowlist (if active) and the per-request `NetworkAccessList` (if present) must
permit a URL for those requests to proceed.

Host-owned system transports are intentionally outside the tenant/agent policy:
system email, utility LLM, LLM provider/model-discovery transports, Daytona
provider APIs, and similar fixed deployment-owned clients are configured by the
deployment, keep their credentials in platform services, and do not route
through `EgressService`. They must not require adding provider endpoints such as
`api.resend.com` to the tenant/agent allowlist.

### Maximum priority (hard ceiling)

The system policy is a separate, AND-ed gate, it is **never merged into**
the harness/agent/session `NetworkAccessList`. Those layers can only narrow
within it (intersection on `allowed`, union on `blocked`); they can **never
widen past it or override it**. When a curated mode is active, a session that
explicitly allows a host still cannot write to it through tenant/agent egress
unless the system allowlist also lists it, and cannot reach a denied host at
all. The system policy always wins for the request kinds it governs.

### fetchkit / web_fetch

When `ToolContext.egress_service` is present (always true in the runtime),
`web_fetch` injects the egress boundary as fetchkit's HTTP transport
(`crates/integrations/src/web_fetch/egress_transport.rs`), so the allowlist is
enforced at the boundary for every hop like any other egress traffic.

On both paths the tool pre-checks the initial URL and returns the distinct
"Endpoint blocked by system policy: …" error, naming which rule refused it,
before any request is made
(`crates/integrations/src/web_fetch/lib.rs`). A denial raised at the egress
boundary itself (e.g. a redirect hop) surfaces as "Outbound request blocked by
network policy: …". On the direct path (contexts without an egress service,
e.g. embedded hosts) the pre-flight check is the only enforcement.

### Operator responsibility

Because enforcement covers tenant/agent runtime traffic, an operator enabling
a curated mode must ensure endpoints reachable through capabilities,
integrations, MCP, plugin fetches, and similar runtime HTTP paths are covered by
a group. Self-hosted or uncommon runtime endpoints not present in the curated
groups will be blocked while the allowlist is enabled. Host-owned system
transports such as email, utility LLM, LLM providers, and Daytona are configured
separately by deployment environment and are not governed by this list.

## Org extensions

A curated mode blocks an organization's writes to its own APIs and MCP servers
unless the curated groups happen to list them. An org extension lets that org
add hosts to the allowlist for its own traffic only.

- **Who grants.** A platform user (`Rule::IsPlatformUser`) turns the right on or
  off per org. It is off by default. The grant is a trust decision about the
  tenant: a granted org can name hosts it controls.
- **Who edits.** The org's admins (`OrgSettingsManage`, the same role that can
  change org settings), and only while granted. Writing without a grant is a 403.
  Revoking keeps the stored list but stops enforcing it; a re-grant restores it.
- **Validation on write.** At most 50 patterns, in `NetworkAccessList` syntax.
  Each must name a public hostname: no IP literals, no `localhost` or non-public
  TLD, nothing the static SSRF checks of `validate_safe_url` refuse, no bare `*`,
  and no wildcard directly over a public suffix (`*.com`, `*.co.uk`; a heuristic
  over common two-letter registries, not the full public suffix list). URL
  prefixes carry no credentials, query, or fragment.
- **Enforcement.** `SystemEgressPolicy::check` takes the extension as
  `extra_allowed`. `DirectEgressService` asks for it only when the policy alone
  would deny a request as not allowlisted and the request's scope names an org,
  so allowlisted traffic, open reads and deny-list hits never trigger a lookup.
  The deny list, the per-request `NetworkAccessList` and DNS-pinned SSRF checks
  still apply. Reads of an org's own hosts in `curated-writes` stay open reads
  and are metered like any other.
- **Resolution and cache.** The resolver is the `OrgEgressAllowlist` contract.
  The server installs a database resolver and the gRPC worker one over the
  internal command `worker_get_org_egress_allowlist`, each process-wide
  (`install_runtime_org_egress_allowlist`), so every runtime egress service
  built with `for_runtime_traffic_from_env()` sees it. Answers are cached per org
  for 60 seconds (`ORG_EGRESS_ALLOWLIST_CACHE_TTL`): an edit or revoke takes
  effect within that window. A failed lookup counts as no extension (fail
  closed) and is not cached.
- **Audit.** A request admitted only by the extension is logged with
  `policy=allowlisted_org`. Grants and edits emit `management.settings.updated`
  audit events.

Storage is two columns on `organization_settings` (migration
`197_org_egress_allowlist_extension.sql`); the API is
`/v1/orgs/{org}/egress-allowlist` and its `/grant` subpath, defined in
`crates/server/src/domains/organizations/egress_allowlist/`. Threat model:
TM-AGENT-036.

## Relationship to other controls

| Control | Scope | Configured by |
|---------|-------|---------------|
| System allowlist | Tenant/agent runtime egress | Maintainers (curated), operator toggles via env |
| Org extension | One org's runtime egress, widening the allowlist only | Platform user grants, org admins edit |
| `NetworkAccessList` | Per harness/agent/session, agent-authored URLs | Users/agents |
| Future Egress Gateway | Network component owning outbound policy | Deployment |

The system allowlist is a precursor to the Egress Gateway's outbound allowlist
described in `knowledge/operations/egress.md`: it gives a single deployment-wide allowlist
today, in-process, ahead of the remote gateway.

## Threat Model

Reinforces **TM-AGENT-018** (outbound URL filtering) with a deployment-wide
backstop that does not depend on per-agent configuration being set correctly,
and is the control for **TM-AGENT-035** (open-signup tenants relaying abuse
through the deployment). SSRF to internal addresses is handled separately and
in every mode (TM-API-008, TM-TOOL-018). The allowlist is a weak exfiltration
control on its own: several allowed hosts accept writes with attacker-supplied
credentials, so per-agent `NetworkAccessList` remains the precise control.
