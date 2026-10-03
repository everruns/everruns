# everruns

## What this codebase does

Everruns is a multi-tenant agent runtime and control plane. Rust services expose the API and run durable agent work; a Next.js application supplies the administrative and chat UI.

- Axum serves organization-scoped REST APIs, SSE event streams, authentication, public metadata, and published-agent protocols.
- Public agent surfaces include AG-UI, A2A, FCP, public chat, generic webhooks, and Slack events/interactivity.
- Stateless workers claim durable task rows through an internal gRPC service; NATS primarily carries wake-up notifications and session event streams.
- Background schedulers handle session work, scheduled triggers, lease cleanup, task reaping, tool-result timeouts, retention, and reporting.
- A Rust CLI and MCP endpoint expose control-plane operations, while runtime ToolRegistry and ToolContext objects expose capabilities to agents.

## Auth shape

Authentication is selected at startup and authorization is enforced mainly through handler extractors and domain-layer callers rather than one global HTTP guard.

- AuthMode supports none, admin, full, and external backends; none grants a stable anonymous administrator only in development and is rejected in non-development environments.
- AuthUser accepts opaque personal access tokens, JWT bearer tokens, legacy API-key forms, or the HttpOnly access-token cookie; refresh cookies are rotated atomically.
- ResolvedOrg and OrgContext validate X-Org-Id, organization cookies, or path IDs against stored membership before constructing an organization-scoped Caller.
- Personal access tokens are random opaque credentials stored as SHA-256 digests; current scope enforcement is effectively all-access within the owning user's memberships.
- MCP, published-agent endpoints, Slack, and generic webhooks use distinct primitives such as McpAuthUser, endpoint auth policies, Slack HMAC verification, and channel secrets.

## Threat model

Tenant users, public channel senders, browsers, remote identity providers, MCP peers, plugin/provider endpoints, repository content, and model-generated tool arguments should be treated as untrusted.

- Cross-organization access is the primary isolation risk: every storage query, resource lookup, SSE subscription, and command must retain the resolved organization boundary.
- The internal worker gRPC credential is highly privileged and shared across organizations; accidental public exposure would permit client-supplied organization IDs to reach Caller::internal paths.
- Public agent channels can create durable work and model spend, making signature checks, endpoint liveness, request limits, deduplication, and rate limits security controls.
- Outbound HTTP, OAuth discovery, JWKS retrieval, MCP servers, git operations, and provider integrations create SSRF and credential-exfiltration risk.
- Model output can select tools and arguments; filesystem, shell, network, connection-secret, and durable-resource tools need workspace boundaries and policy enforcement independent of the model.

## Project-specific patterns to flag

These patterns deserve focused review because they encode the project's principal trust boundaries.

- HTTP handlers that omit AuthUser, ResolvedOrg, or OrgContext, use unscoped lookups, or construct Caller::internal outside a documented internal path.
- Public channel handlers (`endpoint_webhooks`, `endpoint_a2a`, `ag_ui`, `slack_events`) that parse or enqueue a body before token/signature verification, endpoint liveness, or rate limiting.
- Direct outbound clients that bypass validate_safe_url, DNS/public-IP checks, pinned resolution, EgressService, or configured network ACLs.
- Tools registered without required context services, argument-schema validation, permission resolution, workspace confinement, or an appropriate PreToolUseHook.
- Queue consumers that trust NATS payloads as authoritative instead of claiming durable rows with ownership and organization checks.
- UI renderers that bypass sandbox/origin checks, or APIs that expose raw provider, connection, MCP, or agent-identity secrets.

## Known false-positives

Several intentional patterns can resemble vulnerabilities without their surrounding controls.

- Development AuthMode::None deliberately maps an anonymous user to an administrator; startup is intended to fail if this mode is selected outside development.
- SHA-256 is intentionally used for lookup of high-entropy opaque personal access tokens, not for hashing human passwords.
- Published-agent and webhook routes intentionally lack normal AuthUser extraction; review their channel token, signature, endpoint-auth, liveness, and rate-limit controls instead.
- X-Org-Id and the organization cookie are selectors, not authority by themselves; ResolvedOrg and OrgContext are expected to validate membership.
- The Next.js proxy checks only for cookie presence as a navigation convenience; the Rust API and AuthProvider remain the authoritative session validators.
