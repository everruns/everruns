---
title: Worker Base Harness (deprecated)
description: Retired level kept for existing agents; new agents choose Worker, Bashkit Worker or Sandbox Worker.
---

**Worker Base** was the middle of the old single chain (Base, Conversation, Worker Base, Worker). The
[harness tree](/features/harnesses/#built-in-harnesses) replaced it: its files and project
instructions moved into [Worker](/built-ins/harnesses/worker/), and its shell moved into
[Bashkit Worker](/built-ins/harnesses/bashkit-worker/) and
[Sandbox Worker](/built-ins/harnesses/sandbox-worker/).

The row stays active and managed, with its name, ID, parent (Conversation) and capabilities
unchanged, so agents, sessions and adopted harness examples bound to it keep working. Selectors hide
it until you choose Show deprecated.

| Instead of Worker Base | Choose |
|---|---|
| Files and bash, no provider | [Bashkit Worker](/built-ins/harnesses/bashkit-worker/) |
| Files and a real machine | [Sandbox Worker](/built-ins/harnesses/sandbox-worker/) |
| Files and tools, no shell | [Worker](/built-ins/harnesses/worker/) |

In the Framework, `Harness::worker_base()` is deprecated and returns `Harness::bashkit_worker()`.
