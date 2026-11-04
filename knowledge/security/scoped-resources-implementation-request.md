---
type: Design
title: "Implementation Request: Scoped Resources and Credential Capture"
description: "Implementation handoff for common scoped resource access and its adoption by credential bindings, web_fetch, connections, and GitHub."
tags:
  - everruns
  - security
  - resources
  - credentials
---
# Implementation Request: Scoped Resources and Credential Capture

Status: **Approved for implementation and shipping 2026-10-19; qualification in progress.**
Start work in the existing Everruns checkout and follow its `AGENTS.md` and
relevant knowledge. This file is the execution plan; the proposed common
contract is owned by [Scoped Resource Ownership and Access](resource-access.md).
The HTTP requirements are owned by the
[web_fetch implementation request](../execution/web-fetch-credentials-request.md).
The user authorized implementation and shipping in the active workspace task.

## Objective

Implement one ownership and runtime-access approach for connections, credential
bindings, and GitHub integration. Support organization, virtual-user,
agent, and session ownership through a common authorization contract while
retaining each domain's payload and enforcement adapter. Use it to complete
HTTP authentication flows without exposing captured credentials to the model.

Memory is excluded from this implementation. Its ownership, data model, mounts,
storage, authorization, and migration require a separate design and rebuild from
scratch. Leave existing memory behavior and data unchanged. The common contract
may support future resource adapters, but do not implement a memory adapter,
compatibility migration, or memory-specific prerequisites in this work.

## Constraints

- Reuse management permission evaluation and the invocation's verified runtime
  identity. Administration, session ownership, and org membership must not be
  substituted for personal credential authority.
- Use default owner access and explicit bounded sharing grants. Ordinary owner
  use does not need repeated consent; new persistent capture, sharing, and
  widening follow the authorization and consent rules of the owning contract.
- Keep typed constraints and domain enforcement: HTTP destination/placement,
  provider audience, and GitHub repositories/permissions.
- Do not add universal session > agent > user > org fallback or a second OAuth,
  virtual-user, permission, or secret-store implementation.
- Preserve existing identifiers, encrypted values, and intended explicit grants.
  Never backfill broader authority from ambiguous
  ownership. References to the same connection must not copy its tokens.

## Delivery order

Each stage may contain several reviewable PRs. Keep unrelated domains runnable
while their adapters migrate; do not leave parallel authorization writers as
the permanent end state.

1. **Shared foundation.** Introduce the common owner and resource-reference
   types, access records, grant lifecycle, trusted execution-context adapter,
   authorization evaluator, metadata-only audit, and conformance tests. Define
   how owner policy, attachments, grants, and typed domain constraints combine.
2. **Storage and migration.** Add referentially sound scoped ownership and grant
   storage. Plan schema/code rollout so mixed workers cannot write obsolete
   ownership or bypass grants. Preserve IDs and encryption; migrate proven
   explicit assignments and report ambiguous rows for repair.
3. **Credential bindings.** Generalize `agent_mcp_secret_bindings` into
   `credential_bindings`, supporting all four owners and both HTTP and MCP
   parameter targets. Migrate bound session values; ordinary capability-owned
   session secrets remain separate. Route Bashkit and MCP injection through
   shared authorization, with current revisions checked per outgoing request.
4. **web_fetch injection and capture.** Implement the dedicated HTTP request
   linked above, including bounded response reading, form/header injection,
   multiple atomic JSON captures, deterministic sanitation, durable consent,
   secure entry, expiry, and revocation. Use the shared services from stages
   1–3; persistent capture cannot be enabled before consent enforcement.
5. **Connections and GitHub compatibility.** Integrate common access through
   thin adapters around the recently refactored GitHub model. First inspect
   current source, tests, and recent commits; reuse existing acting-identity
   selection, per-use token requests, explicit org-installation choices, and
   consumer routing. Add only the shared ownership/grant checks that are missing.
   Preserve provider refresh, minting, and verified webhook provenance. Follow
   the preservation requirements below; this stage is not a GitHub redesign.
6. **Cutover and acceptance.** Remove superseded runtime authorization paths
   for the participating credential/connection domains,
   close migration gaps, update public contracts and documentation, and prove
   the end-to-end identity, revocation, and secret-capture acceptance bars.

Stage 4 depends on the shared credential services. Provider-backed references
also depend on the connection adapter from stage 5; supply that prerequisite
before enabling those references, rather than duplicating provider logic in
web_fetch. Existing memory consumers are not migrated under stage 5; preserve
their current provider-facing behavior through the connection API as necessary
without changing memory ownership, source-sync logic, or mount authorization.

## Preserve the recent GitHub refactor

The existing GitHub shape is intentional and should remain the domain model.
Recent work already introduced per-use token requests (#4604), consumer
migration (#4651), correct minting-App selection (#4655), the agent GitHub
section (#4665), sandbox credential-scope verification (#4684), and org-linked
shared App installations with explicit agent choice (#4669). These are baseline
behavior, not features to rebuild as part of the common resource work. Recheck
the latest checkout in case further refactor stages have landed.

Preserve:

- Agent-owned Apps, the shared Everruns App, and personal credentials as distinct
  external identities; common ownership must not flatten them into generic keys.
- The agent's explicit choice of its own App or an org-linked shared installation,
  including exact org-connection references. No automatic fallback or forced
  reconnect, and no replacement of an existing bot identity.
- The current per-use acting-identity, repository, and permission contract;
  mint with the App belonging to the selected connection. Retain trusted scope
  evidence and sandbox admission checks rather than inferring scope from token text.
- Installation ownership proof, callback authorization, signed webhook handling,
  and routing only to agents that selected the installation.
- The existing GitHub agent card, org installation setup, API/CLI/MCP flows,
  and exclusion of GitHub installation links from unrelated generic org-account
  selectors. Generic resource access must not make these links search/sandbox
  provider credentials.

Use [GitHub's model](../integrations/github-model.md) together with its current
source and implementation-progress notes. Its initial proposal text includes
unimplemented roadmap items; this request does not authorize completing that
roadmap, migrating legacy personal installations, adding new GitHub channels,
or replacing working setup flows. Any required visible behavior change needs
a concrete explanation of the common-access dependency and separate review.

Before changing GitHub, map each existing ownership/selection check to its
common equivalent and identify the actual gaps. Existing explicit assignments
become equivalent grants; avoid rewriting provider-specific storage merely to
match a generic shape. Demonstrate compatibility using existing regression
coverage plus tests at the new authorization boundary.

## Validation and completion

Run the common access conformance suite for every participating domain, with
adapter-specific tests proving its constraints. Include cross-org and wrong-owner
references, shared-session participants, actual responder selection, Playground,
autonomous and delegated work, grant authority, expiry, concurrent rotation,
revocation, cache isolation, worker restarts, and safe upgrade/cutover.

Demonstrate an org GitHub connection shared with a reviewer for selected
repositories and permissions, and a separately authorized read-only sync use.
Prove the bot does not switch to Alice's personal identity when Alice speaks,
and a personal use cannot borrow the bot's grant. Test restriction enforcement
at the actual token/transport boundary. Check that shared foundation changes
do not alter existing memory behavior; rebuilding memory is not an acceptance
requirement or dependency of this work.

For Shipmail, configure every known credential response field, then prove:
Alice completes setup without claim/assertion/access tokens in tool results,
events, traces, summaries, files, or session exports; she reuses the authorized
connection in another session and another agent while valid; Bob cannot use
it; and Settings revocation blocks the next dispatch with reconnect guidance.
Distinguish fixture coverage from real upstream-flow evidence.

Deliver migrations and source, meaningful regression tests, required contract
exports/SDK/CLI updates, manual UI cases, threat-model updates, updated owning
knowledge, and a public changelog for shipped behavior. Use repository scripts
and CI as the source for exact validation commands. In the completion report,
state which stages landed, tests and real flows verified, remaining migration
or upstream dependencies, and the limits of externally exposed tokens/data.
