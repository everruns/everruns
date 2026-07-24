# Multitenancy

## Purpose

Organizations are Everruns' administrative and isolation boundary. They own
resources, memberships, roles, invitations, policy context, and usage. A user
may belong to several organizations and selects one organization for each
org-scoped request.

This spec owns the isolation model and security invariants. It does not copy
database columns, request structs, repository signatures, protobuf messages, or
route payloads.

## Sources of truth

- [`crates/core/src/organization.rs`](../crates/core/src/organization.rs) owns
  organization identifiers, membership DTOs, and role ordering.
- [`crates/server/src/auth/middleware.rs`](../crates/server/src/auth/middleware.rs)
  owns authenticated organization resolution and membership checks.
- [`crates/server/src/api/users.rs`](../crates/server/src/api/users.rs) owns
  organization switching and the browser selection cookie.
- [`crates/server/src/api/organizations.rs`](../crates/server/src/api/organizations.rs)
  and
  [`crates/server/src/domains/organizations/`](../crates/server/src/domains/organizations/)
  own organization, membership, and invitation behavior.
- [`crates/server/src/domains/org_resolver.rs`](../crates/server/src/domains/org_resolver.rs)
  owns the cross-org resource resolver registry.
- [`crates/server/migrations/`](../crates/server/migrations/) owns table
  definitions, foreign keys, indexes, role storage, and invitation lifecycle.
- [`docs/api/openapi.json`](../docs/api/openapi.json) owns exact public HTTP
  shapes.
- [`crates/server/tests/org_isolation_test.rs`](../crates/server/tests/org_isolation_test.rs)
  and
  [`crates/server/tests/org_invitations_test.rs`](../crates/server/tests/org_invitations_test.rs)
  are executable isolation and invitation contracts.

## Core decisions

### Organization-only ownership

There is no personal resource namespace. Org-scoped resources always resolve to
an organization, including in auth-disabled development mode.

The seeded default organization and anonymous development user keep the same
membership and query paths as authenticated deployments. Well-known IDs and
seed behavior belong to the source and migrations linked above.

### Multiple organizations

Users may belong to multiple organizations. Membership carries a hierarchical
role: owner, admin, or member. The source enum and policy definitions are
authoritative for the exact permission mapping.

Membership, roles, organizations, and invitations are local-database
authoritative. Hosted wrappers may use an external identity provider for login
and user identity, but provider organization claims do not replace Everruns'
membership checks.

### Auth-derived scope

Ordinary resource URLs do not contain the organization. The server derives
scope from the authenticated principal plus explicit request selection:

- personal access tokens may select an organization with the supported header
  or browser cookie; a single membership can be resolved automatically;
- browser sessions use the selected-organization cookie;
- auth-disabled mode uses the default organization.

Every selected organization is validated against the principal's memberships.
Multi-org token callers that omit an explicit selection receive an error rather
than an arbitrary organization.

The browser cookie is server-set and readable by the UI so client state can
stay synchronized. Exact cookie flags, resolution precedence, and error status
codes are security-sensitive implementation details owned by the auth
middleware and users API; do not duplicate them here.

## Isolation invariants

Every org-scoped read and mutation must include organization scope, even when a
public resource identifier is globally unique. A lookup that finds a resource
in another organization is indistinguishable from a missing resource to the
ordinary entity API.

Internal numeric organization IDs are storage and authorization details. Public
APIs, URLs, client logs, and user-facing errors use public prefixed IDs.

These rules apply across all storage implementations. The in-memory backend is
not allowed to weaken isolation for convenience.

Sessions and their dependent records may inherit organization through their
owning resource or may store organization directly as the schema evolves. The
actual join path and foreign keys belong to migrations and repositories; the
invariant is that no access path bypasses organization scope.

Global infrastructure and authentication endpoints are explicitly exempt where
their owning specs require it. A resource is not global merely because it lacks
an organization field in an old document.

## Organizations and membership

Organization creation establishes an owner membership. Role changes and member
removal enforce the hierarchy and preserve at least the required administrative
ownership. Exact request types and allowed transitions live in the
organizations domain and permission policy.

Callers cannot use role management to grant a role above their own authority.
Entity APIs return non-enumerating failures when the target organization is not
a caller membership.

## Invitations

Invitations create pending local membership grants without requiring an email
provider.

The invitation contract is:

- the invited email is normalized consistently at creation and acceptance;
- the raw token is generated securely, returned only where the create flow
  requires it, and never stored or logged;
- only a cryptographic hash is persisted;
- acceptance requires an authenticated principal whose normalized email
  matches the invitation;
- expiry, revocation, prior acceptance, malformed tokens, and email mismatch
  produce stable safe error classifications without disclosing token material;
- at most one pending invitation exists for an organization/email pair;
- accepting is transactional and cannot create duplicate membership.

Email delivery is optional and deployment-owned. Failure or absence of an email
provider leaves the invitation pending and the create operation usable through
copy-link UX. See [`email.md`](email.md).

The invite landing page preserves the destination through authentication and
continues acceptance afterward. Authentication return-path safety is defined in
[`authentication.md`](authentication.md).

## Query and API policy

All org-scoped commands receive organization context from the authenticated
extractor and pass it into storage. HTTP, MCP, gRPC, background work, and
platform capabilities must converge on the same domain policy rather than
constructing an unscoped repository call.

Wrong-organization entity access remains a not-found response to prevent
enumeration. Membership management may return authorization failures only
after the organization itself has been established as visible to the caller.

Public identifiers are validated before storage lookup. Exact formats and
error bodies are defined by typed-ID source and OpenAPI.

## Cross-org direct links

A user who belongs to several organizations may open a direct link for a
resource outside the currently selected organization. Ordinary entity APIs
still return not found; they never search all organizations.

The authenticated resource resolver may map a public resource ID to an owning
organization only when the caller is already a member of that organization.
Therefore it reveals no organization outside the caller's existing membership
set.

Top-level entities with dedicated UI detail routes register a resolver by
public-ID prefix in `crates/server/src/domains/org_resolver.rs`. The registry is
the source of truth for current supported prefixes. Do not keep a copied list
here.

The UI handles a detail-page not-found response by:

1. checking the membership-gated resolver;
2. switching to the returned membership without navigating away;
3. retrying the original detail query;
4. rendering the final not-found state only after fallback is exhausted.

The fallback runs at most once for a resource/current-organization pair to
avoid ping-pong. Generic CRUD hooks own the common integration; bespoke detail
hooks must use the same fallback and fold resolver work into loading state.

Adding a new top-level detail entity requires a storage lookup for its owning
organization, a resolver registration, and UI fallback coverage.

## UI selection

The UI initializes organization context from the authenticated user's
memberships, keeps it synchronized through the switch endpoint, and delays
org-scoped queries until selection is ready. Disabled queries alone do not
represent a loading state, so page-level hooks must include organization
initialization in their loading result.

Switching organizations invalidates or rekeys org-scoped caches. It must not
reuse data fetched under the previous organization.

## Usage and billing

Usage records are attributable to an organization at capture time. Aggregation,
limits, and billing must not infer organization later from mutable user
membership. Exact storage fields and aggregation queries live in the usage
domain and migrations. See [`usage-tracking.md`](usage-tracking.md) and
[`budgeting.md`](budgeting.md).

## Security review checklist

- Every resource lookup is scoped before returning existence or content.
- Internal organization IDs do not cross public boundaries.
- Cookie/header selection is validated against current membership.
- In-memory and PostgreSQL backends implement the same isolation behavior.
- Background and worker paths carry authenticated organization context.
- Invitation tokens, hashes, and email-provider errors do not leak.
- Cross-org resolution returns only caller memberships.
- UI caches include organization identity and clear correctly on switch.

The repository threat register remains in [`threat-model.md`](threat-model.md);
it is intentionally not duplicated here.
