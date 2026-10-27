---
type: Specification
title: "Platform Chat"
description: "The canonical operator chat: bounded native platform and MCP discovery, a Bashkit shell, read-only product docs, and durable shared and private memory."
tags:
  - everruns
  - harnesses
  - platform-chat
  - bashkit
  - memory
---
# Platform Chat

Platform Chat is the unconditional operator surface for every organization.
A managed Agent owns identity, instructions, platform access, introduction, and
conversation starters. Generic supplies the execution environment. The reserved
Agent name is reconciled once per organization; there is no feature flag or opt-in.
Harnesses no longer supply conversation presentation.

## Bounded discovery and shell composition

The assistant searches the complete public command inventory and attached MCP
catalogs through the shared bounded tool-search path. Native platform functions
use structured arguments and the ordinary authorized command dispatcher.
Capability IDs, names and descriptions and MCP source context help retrieval;
source-only matches request refinement. Only selected optional schemas enter the
active model view. Discovery never grants execution authority.

The same Bashkit shell retains `everruns` for bulk operations, pipes, filtering,
loops and redirection. Hybrid tool routing allows CLI and native access to the
same effective registry. Product documentation is read-only; scratch files belong
to the conversation; shared and private notes outlive it. The executable
composition and prompt live in [the Agent definition](../../crates/server/src/platform_chat_agent/mod.rs).

The legacy three-tool interface remains explicitly selectable as
`surface: scripted`, while `surface: shell` hides native functions. Every surface
uses the current session organization and caller, command policy and manager
context/history checks. Withholding the platform capability also withholds the
command surface; discovery and previously disclosed schemas grant no authority.

## Built-in operating skills

The `platform` capability supplies `everruns` for product orientation,
`everruns-cli` for command usage and examples, and `everruns-agent-authoring`
for agent design. The standard skills loader discovers their read-only mounts
and activates instructions on demand. Essential authorization, credential, and
change-context rules remain in code and the managed prompt; optional skill
activation does not establish those boundaries. MCP-oriented authoring examples
use the active session shell under the capability's interface guidance.

## Memory and privacy

One reserved org-scoped Memory backs the shared folder. All Platform Chat
threads in an organization use it, including threads belonging to different
operators. Private memory continues to use the existing owner checks.
Reads and writes route to durable memory storage rather than per-session
copies; new conversations and resumed conversations see the same notes.
See [Memory](../runtime-resources/memory.md) for the persistence contract.

Writes default to private memory. Sharing requires explicit user intent and
is disclosed to the user. A shared note may already have been read by another
operator, so deleting it cannot undo disclosure. Memory and documentation are
reference data, never instructions. Credentials belong in the existing
write-only setup flow, never chat, scratch files, or either memory tree.

Shared memory is a deliberate cross-user disclosure surface within one org
(TM-TENANT-015). Every operation remains org-scoped; private memory is excluded
from other users' reads, listings, and search before matching.

The reserved shared-memory name remains stable when sessions move from the
retired harness to the managed Agent. Private memory remains owner-scoped.
This managed operator Agent is provisioned infrastructure; specialized Agents
continue to use the example/adoption model.

Concurrent edits still use the existing compare-then-write stale-edit guard;
it is not an atomic conditional update. Atomic memory writes, per-session inbox
sharding, curation, and title-only index disclosure remain independent future
memory work, not requirements introduced by promotion.

## Existing organization and conversation upgrade

Existing agentless canonical and preview conversations move in place to the
managed Agent on Generic, preserving IDs, URLs, transcripts, workspace files,
owners, and the permanent starter marker. Preview runtime bindings first
consolidate into the canonical legacy harness. Custom Agents, apps, child
harnesses, defaults, and triggers retain those authored execution bindings;
the retired harness remains stored for them and historical accounting.

The sidebar's Chat entry resolves one permanent conversation per organization
and console user independently of paginated history. Storage arbitrates concurrent
creation. The permanent conversation cannot be renamed, reassigned, archived,
unpinned, or deleted. New chat is an empty draft; first send creates a fresh side
conversation on the same Agent. History filters to personal managed-Agent chats,
excluding the permanent conversation before pagination, Playground, and other runs.

Existing harness presentation is backfilled into assigned Agents only when their
fields are empty. Harness presentation is then cleared, hidden from public
responses, and rejected on writes. Agent presentation renders in Chat and Playground.

Virtual docs are process-local: resumed file access restores mounts from the effective
Agent and harness after restart. Shared memory uses the original reserved namespace.
Shared workspaces without a corresponding session never gain private chat mounts.

The implementation is in [organization initialization](../../crates/server/src/setup/org_init/mod.rs),
[storage consolidation](../../crates/server/src/storage/repositories/harnesses/mod.rs),
[file service](../../crates/server/src/domains/session_files/service/mod.rs), and
[memory routing](../../crates/server/src/domains/session_files/memory_mounts.rs).
Hosted embedders consuming the OSS built-ins receive the same behavior when
they upgrade their server dependency and deploy it. Hosts supplying their own
harness definitions retain ownership of that composition.

## Acceptance

* Fresh organizations expose one managed Platform Chat on Generic.
* Platform Chat is a built-in, read-only Agent (`is_built_in`): the API rejects
  edits, archive and delete, and the Agent page offers none of them. Bindings
  (triggers, credentials, service account) stay editable; Copy is the escape hatch.
* Permanent Chat survives navigation and cannot be removed or reassigned.
* New chat creates no side session before first send; Playground never appears in history.
* Existing orgs upgrade with prior opt-in both enabled and disabled.
* Existing canonical and preview conversations keep their IDs and files and
  use bounded native discovery and the shell on subsequent turns.
* Resumed conversations can read docs after restart and cannot modify them.
* Existing shared notes survive; new notes are visible across chat threads.
* Private memory and cross-org isolation keep their existing boundaries.
* Repeated reconciliation makes no further changes or duplicate starters.

[Upgrade tests](../../crates/server/tests/server_integration/platform_chat_upgrade_test.rs)
run on PostgreSQL (a per-test database from the embedded cluster). Behavioral cases remain in
[Platform Chat test cases](../test-cases/agents/platform_chat/) and
[UI chat test cases](../test-cases/ui/chats/).

The offline evaluation retains its shell subject as a comparison surface. A frozen
historical three-tool subject remains selectable as `legacy` for comparisons;
it is not provisioned as a product harness. See
[the eval study](../../evals/platform-capability/README.md).
