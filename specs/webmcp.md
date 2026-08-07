# WebMCP UI tools

## Purpose

The authenticated Everruns UI exposes a small browser-native tool surface through WebMCP so a
browser agent can navigate and operate the same visible application as the signed-in user. WebMCP
is progressive enhancement: browsers without `document.modelContext` retain the ordinary UI.

This surface complements the first-party backend MCP endpoint. Backend MCP is appropriate for
remote integrations; WebMCP is for a human-supervised browser agent sharing the user's active page,
organization, authentication, and UI state.

## Scope

The first surface contains:

- application context, bounded agent/session search, and controlled navigation;
- starting a session from the active agent page;
- sending a text message from a ready session page;
- cancelling the active turn from a session page.

Tool names use the `everruns_` prefix. Shell tools are registered only inside the authenticated
application. Resource tools are registered only while their owning route and state are active.

The UI does not discover or execute WebMCP tools from third-party pages. It does not expose tools
cross-origin, accept arbitrary navigation URLs, simulate DOM clicks, or make Everruns embeddable.
Declarative form tools are out of scope until the imperative surface has shipped and been reviewed.

## Lifecycle

Registrations use `document.modelContext.registerTool()` with an `AbortSignal`. They are removed on
unmount, route/resource changes, organization changes, logout, or feature disablement. Unsupported
browsers and browser-level permission rejection must not affect the ordinary UI.

Execution callbacks revalidate the current organization, route-bound resource, and applicable
resource state immediately before acting. Model-provided identifiers never override the resource
bound by the current page.

The deployment-wide experimental gate controls the `tools` Permissions Policy. The org-effective
gate controls registration. Production origin-trial tokens are deployment configuration and are
never committed to the repository.

## Tool behavior

Read and navigation tools return concise JSON-safe values. Search is lazy, queries only the
authenticated org-scoped APIs, omits descriptions and message content, caps results, and marks the
returned content as untrusted. Navigation accepts a closed page vocabulary or a validated agent or
session identifier; it never accepts a URL or arbitrary path.

Mutating and billable tools require a visible Everruns confirmation for every invocation. Only one
confirmation may be pending at a time. A route or organization change rejects the pending request.
Starting a session, sending a message, and cancelling a turn are non-idempotent unless their owning
backend contract says otherwise.

The exact tool names, schemas, annotations, and result objects live with the frontend registration
code. They are browser-facing API shapes and require contract tests when changed.

## Security invariants

- Existing authenticated, org-scoped backend APIs remain authoritative for authorization.
- WebMCP is disabled outside the authenticated application and when the org gate is off.
- The Permissions Policy is `tools=(self)` when enabled and `tools=()` otherwise.
- Registrations omit `exposedTo`; cross-origin documents cannot discover or execute the tools.
- Every mutation is confirmed in Everruns with its action and target visible.
- Tool arguments, results, descriptions, and counts are bounded.
- Search results never include prompts, transcripts, credentials, connection configuration, or
  resource descriptions.
- Failures returned to a browser agent do not disclose secrets or unrelated server details.
- Tool annotations accurately describe read, destructive, idempotent, and untrusted-content
  behavior; annotations inform clients but never replace enforcement.

Threats and mitigations are tracked under the Web Security section of `specs/threat-model.md`.

## Success bar

- With the feature and browser API enabled, the expected shell tools are discoverable.
- Route/state changes add and remove the corresponding resource tools without stale registrations.
- A browser agent can search and navigate using the returned resource identifier.
- Mutation execution cannot proceed without an explicit visible confirmation.
- Organization changes cannot execute a callback captured under the previous organization.
- With the flag off or API unavailable, no tools are registered and the UI behaves unchanged.
- Unit tests cover lifecycle and contracts; a real-browser smoke test discovers and executes tools.
