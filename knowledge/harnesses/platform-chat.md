---
type: Specification
title: "Platform Chat"
description: "The canonical operator chat: one Bashkit shell over the everruns CLI, read-only product docs, and durable shared and private memory."
tags:
  - everruns
  - harnesses
  - platform-chat
  - bashkit
  - memory
---
# Platform Chat

Platform Chat is the unconditional operator surface for every organization.
The shell-based implementation formerly called Platform Chat v2 replaces the
three-tool implementation under the existing `platform-chat` identity. There
is one built-in Chat role and no feature flag or organization opt-in.

## One shell, one namespace

The assistant manages the platform through the `everruns` CLI in the same
Bashkit shell it uses for files, pipes, filtering, loops, and redirection.
Product documentation is read-only; scratch files belong to the conversation;
shared and private notes outlive it. The executable composition and prompt
live in [the harness definition](../../crates/server/src/harnesses/platform_chat.rs).

The earlier `discover` / `query` / `execute` split forced the model to reason
about tool selection without providing a permission boundary. Authorization
already belongs to command execution. The shell uses that same command
execution path and its existing policy checks; a different spelling grants
no additional authority. Help is progressively disclosed by CLI node and leaf.

The platform CLI is installed from the session's effective tool registry.
A harness withholding platform capability also withholds the command.
Arguments remain data across shell forwarding, and invocation bounds prevent
one shell loop from amplifying a turn into unbounded control-plane calls.

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

The reserved name remains stable across promotion so earlier preview notes
are reused. Neither a built-in Agent nor a new memory scope is necessary.
A built-in Agent would reopen the duplicate auto-seeding problem that the
example/adoption model intentionally avoids.

Concurrent edits still use the existing compare-then-write stale-edit guard;
it is not an atomic conditional update. Atomic memory writes, per-session inbox
sharding, curation, and title-only index disclosure remain independent future
memory work, not requirements introduced by promotion.

## Existing organization and conversation upgrade

Built-in reconciliation updates the existing canonical row in place, keeping
its ID. Existing chats keep their conversation URLs, history, files, owner,
pins, and starter identity. Starter uniqueness remains enforced by storage.
No replacement starter is minted during promotion.

Reconciliation also moves operational bindings from the former preview to the
canonical harness: sessions, agents, apps, child harnesses, organization
settings, and trigger execution contexts. The preview is then retired from
selection. Its stored row remains for immutable accounting and evaluation
history; historical records are not rewritten. The consolidation is scoped
to built-ins in the same org and is idempotent.

Virtual docs are process-local, so file access restores them from the current
harness after restart. Existing chats lazily resolve or create shared memory;
a new chat is not required to initialize either resource. Shared workspaces
with no corresponding session never gain private or chat memory mounts.

The implementation is in [organization initialization](../../crates/server/src/org_init.rs),
[storage consolidation](../../crates/server/src/storage/repositories/harnesses.rs),
[file service](../../crates/server/src/domains/session_files/service.rs), and
[memory routing](../../crates/server/src/domains/session_files/memory_mounts.rs).
Hosted embedders consuming the OSS built-ins receive the same behavior when
they upgrade their server dependency and deploy it. Hosts supplying their own
harness definitions retain ownership of that composition.

## Acceptance

* Fresh organizations expose one canonical Platform Chat with the Chat role.
* Existing orgs upgrade with prior opt-in both enabled and disabled.
* Existing canonical and preview conversations keep their IDs and files and
  use the shell surface on subsequent turns.
* Resumed conversations can read docs after restart and cannot modify them.
* Existing shared notes survive; new notes are visible across chat threads.
* Private memory and cross-org isolation keep their existing boundaries.
* Repeated reconciliation makes no further changes or duplicate starters.

[Upgrade tests](../../crates/server/tests/server_integration/platform_chat_upgrade_test.rs)
exercise both PostgreSQL and in-memory storage. Behavioral cases remain in
[Platform Chat test cases](../test-cases/agents/platform_chat/) and
[UI chat test cases](../test-cases/ui/chats/).

The offline evaluation defaults to the canonical shell surface. A frozen
historical three-tool subject remains selectable as `legacy` for comparisons;
it is not provisioned as a product harness. See
[the eval study](../../evals/platform-capability/README.md).
