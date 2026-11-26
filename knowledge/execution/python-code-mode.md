---
type: Decision
title: Python Code Mode with Monty
description: Monty is an optional Python execution interface on both Everruns platforms, with isolated workers and private continuation and effect journals.
tags: [execution, framework, security, monty]
---

# Python Code Mode with Monty

Framework and Server are both platforms. Monty is another execution option,
not a platform and not a replacement for Bashkit. Bashkit already provides
Python through CPython WASM; adding Monty is justified by its direct Python
host-call interface and serializable interpreter suspensions. No language-only
split assigns Python work to Monty and shell work to Bashkit automatically.

## Boundaries

The implementation belongs in [integrations](../../crates/integrations/src/monty/mod.rs).
Facade, hosted capability registry and agentd select the same optional feature.
It is not a `SessionSandboxProvider`: there is no arbitrary process, package or
full workspace lifecycle to implement that contract. No remote Monty service
or commercial-server dependency is required.

A disposable subprocess protects the host from interpreter crashes. The
[Linux launcher](../../crates/integrations/src/monty/isolation.rs) adds Landlock,
seccomp and kernel limits; unsupported hosts fail closed. A trusted custom host
may choose a different isolated launcher explicitly. The interpreter has no
host mounts, socket adapter, raw credential material or automatic shell fallback.
All effects use existing tools, their current turn allowlist and policy chains.
Missing egress is explicitly disabled, preventing standalone transport fallback.

Monty 1.5.0 is pinned with its subprocess protocol and types to retain the
repository's Rust 1.96 contract; 1.6 and 1.7 require Rust 1.98. Upgrade the host
crate, runtime binary, snapshot version and qualification together. Parser
crashes remain contained in disposable workers; a process boundary alone is
not an OS privilege boundary.

## Continuations and effects

A [neutral host-private journal](../../crates/contracts/src/runtime/durability.rs)
uses the existing durable claim/settle contract. The engine binds it to the
session and admitted input. The [bridge](../../crates/integrations/src/monty/nested.rs)
records a snapshot before invoking a tool and the policy-filtered result before
returning to Python. The initial invocation is replay-safe because every effect
has a stable journal identity; unsettled effects fail closed rather than repeat.
New programs cannot invoke effects without the host's stable invocation
idempotency key. Pure calculations do not need this identity.

A hosted approval returns the normal approval payload plus a continuation
reference. After the person answers, the agent explicitly calls `python` with
`resume`. That restores the suspended program, rechecks current permissions
and preserves the pending call's exact identity. It does not rerun source or
silently perform the operation directly. Rejection cannot execute the pending
call. This composes with existing approval APIs and avoids adding fake nested
assistant/tool messages to conversation history.

A Monty snapshot alone is not durable execution. Side effects need separate
claims and results; an unknown outcome still requires reconciliation. Completed
results can be replayed. Interpreter-version changes reject old snapshots.
Saved suspension counts and active-time budgets carry across resume; wall time
waiting for a person is excluded. Bounds apply independently to source, output,
JSON conversions, protocol frames, interpreter memory and snapshots.

Server uses its private durable result store. Separate workers reach it through
an [internal session-scoped command](../../crates/server/src/domains/code_execution.rs),
which verifies organization ownership and the code namespace, rejects forced
settlements, and excludes snapshots from public commands and change history.
This binding does not change outer tool replay semantics. agentd's
[code journal](../../crates/agentd/src/session_code_journal.rs) is session-owned
replicated state and has a storage quota; it follows session moves and snapshots
without entering the public feed. Framework can use a private file journal or
an injected host store. Persisting this journal does not make Engine's default
in-memory session authority durable. Retain records while a session or unsettled
effect can still resume; deleting the session retires agentd records.

## Composition and qualification

Native schemas remain visible; this version is not a large-catalog compression
feature. Discovery and invocation use the same eligibility predicate and current
allowlist. Execution, client-side and turn-control tools stay direct. Deferred
MCP tools must first enter the current registry through native discovery.
Bashkit's CPython WASM selection and allocator are unchanged. Lua and shell
routing must preserve the Python execution entry point.

[Runtime tests](../../crates/integrations/src/monty/tests.rs) cover actual Monty
execution, approval/resume across fresh workers, recorded effect replay,
unknown-effect refusal, revocation, structured values, limits, network failure
and Bashkit coexistence. The runtime qualification belongs to its dedicated CI job,
not the generic shell-helper test inventory: compiling the interpreter there
duplicates work and can exhaust the shell job budget. [Qualification](../../.github/scripts/qualify-monty.sh) also runs
kernel restriction probes; inability to install Landlock is a failed deployment
qualification, not a skipped or weaker sandbox. The CI job is scoped to this
surface and uses a separate Rust cache key.

The [Code Mode comparison](../../evals/shell-code-mode/README.md) uses identical
fixtures across native, CLI and Monty, with actual-effect and surface-attribution
scoring. Its approval cases require a single source invocation followed by saved
resume, and reject duplicate reservation effects or execution before the decision.
The [MCP scale study](../../evals/mcp-scale/README.md) also has a Monty arm, preserving
its actual native-schema visibility. Model quality, runtime correctness and hosted
qualification are distinct evidence: interpreter-only results cannot qualify the
Linux launcher, and Framework results cannot qualify Server/agentd failover.
The [hosted recovery runner](../../evals/shell-code-mode/hosted_recovery.py)
requires a real pending approval, process replacement and authoritative effect
read-back. Hosted qualification disables the file-journal fallback; cluster
qualification also replaces the Raft leader before answering the approval.

Public guidance: [capability](../../docs/capabilities/python-code-mode.md),
[Framework example](../../docs/framework/python-code-mode.md), and
[choosing options](../../docs/framework/execution-options.md).
