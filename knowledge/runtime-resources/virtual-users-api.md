---
type: Specification
title: "Virtual Users API"
description: "API contract for virtual users: one resource, management and consumer authorities, links to route and schema owners."
tags:
  - everruns
  - runtime-resources
  - identity
---
# Virtual Users API

Status: implemented. Virtual users are organization scoped.
The [identity design](virtual-users.md)
owns the domain boundaries and migration rationale. The [HTTP handlers](../../crates/server/src/api/virtual_users.rs), [connection handlers](../../crates/server/src/api/virtual_user_connections.rs),
and [generated OpenAPI](../../docs/api/openapi.json) own the exact request and response shapes.

## One resource, independent authorities

`VirtualUser` is the canonical runtime account resource. Management clients and
consumer clients use the same domain commands, connection service, and store.
Different permissions expose different actions and safe projections.

Management authentication remains on the existing `/v1/auth/*`, management
user/profile, organization membership, and PAT surfaces. Console preferences
remain management-account state; agent-facing preferences move to virtual users.

Every operation resolves a typed authority: authenticated management actor,
verified runtime subject, or an explicitly authorized combination. A missing
management account must never become `Caller::internal` or acquire a default
management role. Policies run at the shared command boundary for HTTP, MCP,
CLI, platform commands, and worker-mediated operations.

## Resource routes

| Route | Operations | Authorization and meaning |
|---|---|---|
| `/v1/virtual-users/config` | GET | Management capability hints, following existing resource-config conventions. |
| `/v1/virtual-users` | GET, POST | Org-scoped management listing/creation. Listing is paginated, with usage, archived state, and search filters. Consumer credentials cannot enumerate the org. |
| `/v1/virtual-users/{id}` | GET, PATCH, DELETE | Authorized resource read, profile update, and archive. Self-service permits profile fields only; management permissions control lifecycle. |
| `/v1/virtual-users/{id}/delete` | POST | Explicit destructive deletion, following existing lifecycle conventions. Separate dangerous-action policy. |
| `/v1/virtual-users/me` | GET, PATCH | Console resolves the default end-user virtual user in its selected org; runtime auth resolves its verified subject. PATCH changes only the runtime profile. |
| `/v1/virtual-users/{id}/preferences` | GET | Authorized agent-facing preferences. Per-key GET/PUT/DELETE uses `/preferences/{key}`. |
| `/v1/virtual-users/{id}/connections` | GET | Sanitized grant metadata. No access or refresh tokens returned. |
| `/v1/virtual-users/{id}/connections/{provider}` | POST, DELETE | Validate/store an API-key grant or disconnect it. Self-service for end users; explicit management authorization for service accounts. |
| `/v1/virtual-users/{id}/connections/{provider}/verify` | POST | Verify the stored grant without returning its secret. |
| `/v1/virtual-users/{id}/connections/{provider}/authorize` | GET, POST | Start target-bound setup. GET redirects console browsers; POST returns an authorization URL for bearer-authenticated clients. |
| `/v1/connection-callbacks/{provider}` | GET | Provider callback resolves its target from verified, one-time setup state, not the current org/account selection. |
| `/v1/connection-providers` | GET | Safe provider availability in the authorized org/runtime scope. External consumers see only providers allowed by their endpoint. |
| `/v1/virtual-users/{id}/bindings` | GET | Sanitized linked identity metadata under self/management policy. |
| `/v1/virtual-users/{id}/bindings/{binding_id}` | DELETE | Authorized unlink, preserving required default/provenance constraints and revoking affected runtime authority. |

`/v1/virtual-users/me/connections` and `/me/preferences` support the same
subresource operations as the ID-based routes. They resolve `me` once and
dispatch to the identical commands. These shortcuts create no separate resource,
business logic, or grant store. Static paths such as `me` and `config` must not
be parsed as entity IDs.

The self response can include allowed actions, so consumer UI need not call the
management config endpoint. All ID-based operations enforce org scope and
subject/action policy; possession of a virtual-user ID is not authorization.
Profile administration never automatically grants access to private connections.
End-user/service usage conversion is excluded from ordinary PATCH in this work.

## Identity establishment and linking

Console self bindings are provisioned idempotently from authenticated account
and org membership. Existing endpoint/channel authentication resolves verified
issuer/realm/subject to a binding, then to a virtual user. It must not accept an
unverified `external_actor` payload as proof of identity.

Browser self-service outside the console uses
`POST /v1/channels/{channel_id}/runtime-auth`: exchange the endpoint's verified
consumer authentication for a bounded runtime session/token. It binds the org,
virtual user, endpoint, audience, expiry, and allowed actions/session scope.
Shared application tokens do not identify a person. Anonymous ingress uses a bounded visitor cookie and cannot claim another consumer's account. Runtime self-service setup currently requires verified endpoint authentication.
Reuse the existing endpoint verifiers; no new consumer IdP is required.

Identity linking requires proof of the new identity and authorization to the
target virtual user. An implementation can expose a typed link-start/link-complete
flow under the bindings resource. Generic public POST of an arbitrary issuer
and subject must not mint a trusted binding. A trusted application integration
may resolve/provision customer bindings only in its authorized org and realm;
its credential must not assert Slack/OIDC identities from another namespace.

Runtime credentials are accepted only on permitted consumer actions, not auth,
PAT, management-list, agent-edit, or org-administration routes. Endpoint-scoped
credentials cannot cross to another endpoint or agent merely because both are
in the same org. Provider setup capabilities are narrower still: target,
provider, purpose, org, lifetime, and optional invocation/endpoint are explicit.

## Agents, sessions, and turns

Keep the existing Agent and Session resource families.

- Agent create/update exposes an explicit `service_virtual_user_id` binding.
  The server checks org, active state, service usage, and management permission.
  It replaces the old agent-identity relation rather than adding a parallel one.
- Session list supports `source=chat&mine=true`; `mine` refers to the resolved
  runtime subject. Management filtering by virtual user is a separate authorized
  filter, never an access grant. Pins use the requesting virtual user.
- Session creation, messages, participants, cancel, fork, events, SSE, and
  approval/question-answer paths keep their resource identity. Runtime auth
  checks authorized endpoint/session access in addition to subject identity.
- Creating a message resolves its initiator server-side from verified auth.
  Ordinary callers do not choose `acting_virtual_user_id` or use arbitrary
  message metadata to select a credential owner. Authorized backend delegation
  must carry an explicit scoped grant for the claimed subject.
- Responses/events attribute the initiating virtual user and selected acting
  principal. The actor for an external operation may differ from the initiator;
  service usage comes from the active responder's binding and declared tool policy.
- Platform Chat management commands require explicit, revalidated Everruns-user
  authority. Virtual-user self auth and session ownership do not supply it.

Existing `/v1/channels/{channel_id}/...` ingress protocols stay in place. They call
the same subject resolver and execution commands; no separate virtual-user chat
storage or second sessions API is introduced. Preserve each ingress protocol's
sanitized responses and event projection.

## Worker boundary and compatibility

Internal credential resolution takes a validated org/session/turn/operation
reference. The server derives the initiator, responder, credential usage, and
connection subject from stored context and effective tool configuration. A
worker-supplied subject or `acts_as` value alone cannot grant authority. Cleanup uses the resource's organization-scoped runtime owner and provider. Pending migrations block cleanup until that owner has an explicit grant.
Provider tokens remain confined to trusted execution components.

`/v1/user/connections/*` and legacy provider callback
URLs can be read/request adapters during cutover. All writes dispatch to the
canonical virtual-user services. Old callback state must either resolve through
an explicit migration mapping or expire and restart setup; it cannot write into
an independent legacy store. SDK, CLI, MCP catalogs, OpenAPI, and protobuf move
to the same contract. Retire legacy names after callers migrate.

Responses use existing error/lifecycle conventions and avoid disclosing other
orgs' resources. OpenAPI documents management and runtime authentication and
action requirements separately. Acceptance is defined in the identity design,
including cross-org rejection and no legacy runtime ownership fallback.
