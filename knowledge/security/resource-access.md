---
type: Design
title: "Scoped Resource Ownership and Access"
description: "Common ownership, grants, and runtime authorization for connections, credential bindings, and GitHub access, extensible to future resources."
tags:
  - everruns
  - security
  - resources
  - identity
---
# Scoped Resource Ownership and Access

Status: **Approved for implementation 2026-10-19; qualification in progress.**
This contract governs the scoped credential and connection access implementation.

Current scope: connections, credential bindings, and GitHub integration.
**Memory is deferred for a separate design and rebuild from scratch.** Its
ownership, mounts, storage, authorization, and data migration are excluded;
existing memory behavior and data remain unchanged. The common contract can
support future resource kinds without adapting the current memory implementation.

## Problem

Connections, bound credentials, and memory each answer similar questions with
different machinery: who owns a resource, which run may use it, what that use
permits, and how access ends. GitHub needs the same answers across tools, MCP,
repository sync, triggers, and sandbox git. Repeating those decisions per
integration creates inconsistent identity selection and revocation.

The common approach must preserve the distinction between management accounts,
runtime virtual users, agents as behavior, and external provider identities.
An org-owned GitHub installation may act as the Everruns bot; ownership alone
does not tell us who appears on GitHub.

## Existing foundations and gaps

- [Virtual users](../runtime-resources/virtual-users.md) own runtime identity.
  The persisted invocation identifies the verified consumer and actual responder;
  durable ownership and management permissions do not supply personal authority.
- [Permissions](permissions.md) already provide domain management policies
  through the shared command runner and permission resolver.
- [Connections](../integrations/user-connections.md) have a canonical
  virtual-user store, encryption, OAuth refresh, and provider adapters. Its older
  global-user and session-owner prose is superseded by the virtual-user contract.
- [Bound secrets](bound-secrets.md) use session storage and agent credential
  bindings. Their HTTP bindings already protect Bashkit requests.
- [Memory](../runtime-resources/memory.md) has org, agent, and user ownership.
  Private user mounts still use the legacy management-owner storage key and
  session/workspace routing. They are not an invocation-based authorization
  implementation and must not be generalized by copying that lookup.
- [GitHub](../integrations/github-model.md) already has explicit acting identity
  and per-use repository/permission requests in parts of the implementation.
  Its design contains both planned behavior and implementation progress; source
  remains authoritative for each migrated path.

GitHub has recently undergone a substantial refactor, and its existing shape
is the accepted domain baseline for this proposal. Preserve distinct App/person
identities, explicit shared-installation choice, per-use token narrowing, setup
surfaces, and verified event routing. Common access integrates through adapters
and equivalent grants, not a second GitHub redesign. The
[implementation request](scoped-resources-implementation-request.md) records
the preservation and compatibility checks for stage 5.

## One resource access contract

Connections and credential bindings retain their own payloads and
lifecycles. They share ownership types, access grants, authorization decisions,
revocation rules, and audit vocabulary.

| Concept | Meaning |
| --- | --- |
| Organization | Mandatory tenant boundary for the resource and every access. |
| Owner | Exactly one organization, virtual user, agent, or session. |
| Resource reference | Stable, typed identity of a connection or credential binding; future resource kinds may opt in. |
| Access grant | An authorized owner's permission for a named consumer to perform bounded operations. |
| Attachment | Configuration selecting a resource for a feature, such as an HTTP binding or GitHub use. |
| Acting identity | The verified runtime identity on whose behalf the operation executes. |

The shared owner model supports all four scopes. A domain may support a subset;
future resources need not fill every scope. Virtual-user owners may be end users
or service accounts, preserving
their existing authorization distinction.

An attachment selects a resource but cannot manufacture authority. Creating an
attachment and its necessary grant may be one authorized operation. A model
editing ordinary configuration, a prompt, or session KV cannot create a grant.

## Default access and explicit sharing

Owner access is evaluated through a common policy: the invocation must prove
the owner relationship, the operation must be allowed, and the domain's exposure
rules must pass. It does not require a separate consent prompt on every request.

- Virtual-user resources follow that exact verified identity, in the same org.
- Agent resources follow authorized execution by that agent; a guest cannot
  borrow the host's credentials simply by sharing its session.
- Session resources follow the authorized session and acting subject/usage.
  Personal grants in a shared session remain personal, rather than becoming
  available to every participant.
- Org resources are available to consumers selected by org policy or explicit
  grants. Merely belonging to the org does not expose every secret or mount.

Sharing with another owner is explicit. Examples are an org connection granted
to an agent for selected GitHub repositories or a person's connection delegated
for a specific session.
An org-wide service grant may explicitly authorize all eligible agents; the
system need not require one manually created row per agent.

The grant records the authorizer, consumer, allowed operations, typed domain
constraints, expiry, revocation revision, and permitted execution context.
Permission to use does not imply permission to manage, re-share, or widen access.
Re-sharing and ownership transfer are outside the first implementation.
Delegated child work receives only an explicitly permitted subset of its
parent's access and cannot turn a session grant into durable agent access.

Private access is not safe merely because a token stays hidden. A grant must
also permit the exposure context: a personal memory file or API response may
disclose private data into a shared transcript or workspace. Private resources
remain excluded from shared exposure unless their policy explicitly allows it.
Simulation/Playground restrictions continue to apply.

## Resolution and authorization

Every runtime use follows one sequence:

1. Load the trusted invocation or background-job context. Derive org, session,
   responder, initiating virtual user, service identity, and allowed exposure.
   Do not accept those authorities from model arguments.
2. Select the acting identity and exact attachment/resource. Retain the current
   `user`, `service`, and `none` vocabulary. Existing explicit
   `user_or_service` behavior remains an opt-in selection policy with recorded
   outcomes, not a default for new integrations or a way around denied access.
3. Verify resource ownership, tenant, lifecycle, owner access or an applicable
   grant, and requested operations/constraints. A single authorization path
   must cover the use; unrelated grants cannot be stitched into broader access.
4. Apply the domain adapter. It narrows a provider token or injects a bound value
   only after authorization. A request exceeding its
   effective permissions is denied, not silently performed with broader rights.
5. Audit resource, owner, grant, initiator, acting identity, operation, and
   outcome without recording secrets or private payloads.

Effective access is the intersection of the grant, attachment configuration,
requested use, domain restrictions, and the upstream provider's actual rights.
Constraints are typed by domain, not arbitrary expressions interpreted by the
generic evaluator.

There is no universal session > agent > user > org fallback. Named overrides
are allowed only within an already authorized identity and declared selection
policy. Denied, expired, revoked, and ambiguous resources fail closed; they do
not cause the resolver to borrow another identity's grant.

Autonomous jobs use a service identity or a bounded persisted delegation.
They never infer personal authority from resource ownership or the last speaker.
Periodic personal use needs explicit consent naming the consumer, purpose,
duration, and unattended use; a successful interactive turn is not that consent.

## Domain adapters

| Resource | Shared authorization | Domain enforcement |
| --- | --- | --- |
| Connection | Owner and authorized consumer; connect, use, revoke | OAuth/API-key lifecycle, provider audience, refresh, external identity. |
| Credential binding | Owner and authorized consumer; use, provision, revoke | Secret source, URL/path, header/form/MCP parameter, write-only injection and redaction. |
| GitHub connection use | Connection grant plus selected acting identity | Installation/account, repositories, GitHub permissions, token minting, webhook provenance. |

A future memory rebuild may adopt this contract for read/write and exposure
checks, but no memory adapter or migration is part of this implementation.

Credential bindings may own an encrypted write-only value or reference an
existing connection. An authorized binding can only restrict its source
connection's access; it cannot expand its audience, repositories, or rights.
OAuth tokens are not copied into a second permanent store to make sharing work.
GitHub remains a connection provider, not another generic credential store.

For example, an admin connects the shared GitHub App at org scope and grants a
reviewer agent access to `acme/app`, with contents read and pull-request write.
The reviewer acts as the configured bot whether Alice or Bob starts it. A
background repository sync can use the same org connection under a separate read-only grant
bound to that background use. A personal GitHub use explicitly selects Alice's
identity and cannot inherit the reviewer's bot grant.

Installation tokens can be narrowed at minting. Personal tokens and some MCP
or sandbox transports cannot enforce the same restrictions once a broad token
is handed over. Such paths must use a mediated operation/proxy or refuse the
restricted exposure; storing a repository constraint is not enforcement.
Existing host allowlists for connection-backed MCP remain mandatory.

Verified GitHub webhook delivery identifies an installation and repository,
then routes only to explicitly attached consumers. Outbound credential access
does not authorize inbound events to impersonate a personal virtual user.

## Storage and revocation

Use a shared access record for org and typed owner identity, with domain
resources referencing it and common grants referencing that same record.
Keep provider data, encrypted values, and binding targets in
their owning domains. Preserve existing public resource identifiers.
Foreign keys and transactional creation/deletion must prevent dangling grants
and cross-org owner/resource links; free-form owner IDs are insufficient.

`credential_bindings` is the generic replacement for
`agent_mcp_secret_bindings`. Migrate bound session secrets into that binding
domain; ordinary capability-owned session secrets remain separate. Connections
continue to use the canonical connection lifecycle. Existing org connection
wrappers become adapters to the common ownership contract rather than a second
source of permission. The exact schema and API live with implementation.

Check current grant/resource revisions before each new participating credential
or connection use, including Bashkit HTTP requests within a long shell call.
Cache keys include tenant, owner, acting identity, resource, grant revision,
and applicable constraints; cached tokens and access handles cannot bypass current
authorization. Existing memory mount caching is not changed by this work.
Rotation uses revision checks so it cannot resurrect a revoked grant.
A missing authorization store denies the use.

Revocation blocks subsequent platform-authorized operations. It cannot retract
data already shown, files already copied, requests already dispatched, or an
externally valid token previously exposed to a sandbox. Prefer mediated use
and short-lived narrowed tokens, with upstream revocation where supported.

## Delivery and acceptance

Deliver incrementally, without a permanent second authorization system:

1. Add the common owner/reference types, grant lifecycle, trusted execution
   context adapter, evaluator, and an authorization conformance suite. Reuse
   the existing permission resolver for management and grant administration.
2. Generalize credential bindings across four owners, migrate existing bound
   values, and route Bashkit, web_fetch, and MCP parameter injection through
   the same access checks. Capture, secure entry, and persistent consent remain
   part of the [bound-secret](bound-secrets.md) extension under design.
3. Adapt connections and GitHub consumers through thin compatibility adapters.
   Reuse existing exact connection references, per-use token requests, and App
   behavior; add the missing shared access checks. Preserve refresh and minting;
   do not silently migrate personal connections to shared service access or
   include unfinished GitHub roadmap work in this migration.
4. Retire superseded credential/connection lookup paths and migrate existing
   explicit configurations into equivalent grants. Ambiguous authority requires repair; migration never
   converts org membership or management ownership into a broader runtime grant.

Each domain runs the common tests for cross-org references, owner checks,
grant creation authority, no implicit identity switching, wrong responder,
shared/private exposure, delegation narrowing, expiry, revocation, restart,
concurrency, and cache isolation. Adapter tests additionally prove URL and form
constraints and GitHub repository/permission enforcement. Existing explicit
attachments preserve their intended behavior. Memory is excluded from migration
and conformance adoption; verify only that existing memory consumers do not
regress due to changes in shared dependencies.

The Shipmail completion bar remains: capture every configured claim token,
assertion, and access token before model-visible processing; reuse the correct
person's connection across agents; revoke through Settings; and find no known
credential values in tool results, events, traces, files, or session exports.

## See also

- [Scoped resources implementation request](scoped-resources-implementation-request.md)
- [Virtual-user execution authority](../runtime-resources/virtual-users.md)
- [Permissions Model](permissions.md)
- [Bound Secrets](bound-secrets.md)
- [Secure web_fetch credentials implementation request](../execution/web-fetch-credentials-request.md)
- [Memory Specification](../runtime-resources/memory.md)
- [GitHub Connection Model](../integrations/github-model.md)
- [Connection-backed MCP audience checks](../../crates/server/src/domains/mcp_servers/connection_backed.rs)
- [GitHub per-use resolution](../../crates/server/src/storage/connection_resolver/github.rs)
- [Current memory mount routing](../../crates/server/src/domains/session_files/memory_mounts.rs)
