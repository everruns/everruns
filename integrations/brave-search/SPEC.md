# Brave Search Capability Specification

## Intent

Brave Search supplies current web-search evidence to agents in both Framework
and hosted execution. It is stateless apart from credentials. Hosted catalog
discovery remains experimental and restricted to development deployments;
Framework applications explicitly select the capability.

## Execution across contexts

The [Framework adapter](src/framework.rs) implements the neutral capability-author
contract and captures an application-owned client. The [hosted adapter](src/tools.rs)
resolves credentials from the current session. Both share the [tool protocol and
search operation](src/search.rs) and [HTTP client](src/client.rs), so requests and
result mapping cannot evolve independently.

Application credentials are read at construction and never serialized into
capability configuration or metadata. Hosted credentials resolve lazily through
user connections, with session-secret fallback. User connections are preferred
because they can be refreshed and reused across authorized sessions. Missing
credentials produce an explicit failure rather than an unauthenticated request.

The hosted feature separates connector UI and inventory discovery from the
Framework dependency graph. The [crate manifest](Cargo.toml) owns feature
selection; [registration](src/lib.rs) owns deployment discovery. The
[connector](src/connection.rs) owns hosted credential entry and validation.
Neither adapter requires per-search resource provisioning or cleanup.

## Security and errors

Both adapters use the same direct HTTP client. Framework calls carry the
application's network authority and do not acquire hosted tenant or connection
authority. Keys remain private client state and are excluded from diagnostic
capability values. Upstream error bodies are omitted because they may echo
request credentials; HTTP status remains available for actionable errors.

Search results are untrusted tool output. Search queries are disclosed to Brave;
applications are responsible for deciding which information may be sent. See the
[Brave threat assessment](../../knowledge/security/threat-model.md#20-brave-search-tm-llm).

## Acceptance evidence

- [Framework acceptance tests](tests/framework.rs) cover public Engine execution
  against mock HTTP, protocol parity, result persistence, and credential separation.
- [Hosted tool tests](src/tools.rs) cover connection precedence, secret fallback,
  missing credentials, and argument handling.
- [Client tests](src/client.rs) cover response parsing, empty results, HTTP errors,
  and malformed responses.
- [Registration tests](tests/plugin_registration.rs) cover hosted discovery and
  development/production gating.
- [Live API tests](tests/smoke_real_api.rs) exercise Brave with explicit credentials;
  the [integration workflow](../../.github/workflows/brave-search-integration.yml)
  owns secret-safe PR and main-branch execution.

Application setup and commands live in the [README](README.md).
