---
type: Specification
title: "Platform Chat"
description: "The canonical operator chat with bounded native platform functions, Bashkit shell composition, read-only docs, and durable shared and private memory."
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
conversation starters. Bashkit Worker supplies the execution environment. The reserved
Agent name is reconciled once per organization; there is no feature flag or opt-in.
Harnesses no longer supply conversation presentation.

## Native operations and shell composition

The managed Agent opts into `{"surface":"native"}` for the Platform capability.
This registers all 350 public, noninternal, nondurable inventory operations as
native `platform_<wire_name>` functions; omitting `surface` still selects the
legacy `discover` / `query` / `execute` interface. The assistant uses those
native functions for individual platform operations and the `everruns` CLI in
its Bashkit shell for files,
pipes, filtering, loops, and redirection. Bounded `tool_search` uses threshold
1 and reveals operation schemas on demand within the configured count and byte
limits. The portable `operations.json` catalog is generated from the server
inventory; workers do not link the server crate. Dispatch checks current feature
flags and permissions. The static metadata spans feature flags, like the local
CLI grammar; a disabled operation can be found but is refused when called.
Native functions and the CLI reach the same authorized
platform dispatch path. Bashkit Worker remains the shell environment.
Product documentation is read-only; scratch files belong to the conversation;
shared and private notes outlive it. The executable composition and prompt
live in [the Agent definition](../../crates/server/src/platform_chat_agent/mod.rs).

The earlier `discover` / `query` / `execute` split forced the model to reason
about tool selection without providing a permission boundary. Authorization
already belongs to operation execution. The shell and native functions use
that same authorized dispatch path; a different spelling grants no additional
authority. Native calls wrap the canonical typed payload in `params`, with
change `reason` and `context_revision` carried as separate metadata. Optional
schema disclosure is bounded by the configured search count and byte limits.

The platform CLI is installed from the session's effective tool registry.
Withholding platform capability from the effective Agent and harness composition also withholds the command.
Arguments remain data across shell forwarding, and invocation bounds prevent
one shell loop from amplifying a turn into unbounded control-plane calls.

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
managed Agent on Bashkit Worker, preserving IDs, URLs, transcripts, workspace files,
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
  use bounded native functions with shell composition on subsequent turns.
* Resumed conversations can read docs after restart and cannot modify them.
* Existing shared notes survive; new notes are visible across chat threads.
* Private memory and cross-org isolation keep their existing boundaries.
* Repeated reconciliation makes no further changes or duplicate starters.

[Upgrade tests](../../crates/server/tests/server_integration/platform_chat_upgrade_test.rs)
run on PostgreSQL (a per-test database from the embedded cluster). Behavioral cases remain in
[Platform Chat test cases](../test-cases/agents/platform_chat/) and
[UI chat test cases](../test-cases/ui/chats/).

Hosted evaluation classifies native calls through their canonical operation
metadata, including the production function prefix, alongside CLI traces.

The offline evaluation defaults to the captured historical CLI surface. A frozen
historical three-tool subject remains selectable as `legacy` for comparisons;
it is not provisioned as a product harness. See
[the eval study](../../evals/platform-capability/README.md).

A separate `surface-comparison` experiment compares CLI, eager native and bounded
native discovery against the same captured platform catalog, in-process state
and policy prompt. Native functions wrap canonical payloads with separate change
reason/context-revision metadata. Completed-operation evidence, ordered reads and
exact state differences grade mutations. Bounded discovery uses the production
index, while its offline HTTP transport sends changing top-level tool lists;
it does not qualify Framework durable replay or provider inline tool changes.
Separate large-result tasks measure filtering and aggregation.

The historical corrected Muse comparison used an offline, unprefixed 344-tool
catalog including 11 durable operations; production exposes 350 native
operations with the `platform_` prefix. Keep those measurements and limitations
as historical evidence; they do not qualify production models, domain
authorization, transactions, hosted persistence, or provider inline tool
changes. Production schema discovery is bounded, and Bashkit Worker remains
available for shell aggregation. Do not infer provider cache hits from inline
tool changes; qualified drivers preserve the baseline where supported, while
fallbacks may rewrite the tool list.
The original two-arm comparison remains provisional historical evidence because
it lacked invocation-envelope and complete state assertions; see the
[recorded findings](../../evals/platform-capability/README.md#follow-up-findings-2026-10-24)
and corrected runs in the study before comparing surface costs.

The corrected evidence supports keeping both execution mechanisms available:
bounded native lookup can avoid CLI help, while rich platform write schemas and
large result aggregation favor shell composition in this corpus. Bounded platform
discovery still has a recorded search-budget miss. Selecting native functions
for the managed composition does not turn that fixture run into production
qualification. The [recorded comparison](../../evals/platform-capability/README.md#corrected-muse-comparison-2026-10-24)
owns measurements and the HTTP/cache limitations; the
[provenance](../../evals/platform-capability/results/corrected-2026-10-24-summary.json)
separates cohorts and preserves failed attempts.
