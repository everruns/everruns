---
type: Specification
title: "Suggested fixes for findings"
description: "How the Inspector proposes an instruction change for a finding, how a person checks it against the agent's evals, and when it may be applied."
tags:
  - everruns
  - evaluation
  - detection
---

# Suggested fixes for findings

Status: implemented (Evals rebuild, step 10). Self-healing, which acts without
a person for protective actions, is the next step and builds on this one.

## What it is

When the Inspector writes up a finding about an agent's work (agent stuck,
followed an injected instruction, runaway schedule), it may also propose a
**fix**: one short addition to the agent's instructions that would have
prevented what the finding shows. The fix appears on the finding page as the
"Suggested fix" card of the approved mockup.

A fix moves through: `proposed` → `checking` → `verified` or `regressed` (or
`check_failed`) → `applied` or `rejected`. Editing the text sends it back to
`proposed`.

## Contracts

- **Nothing changes the agent without a person.** Applying is the only step
  that edits the agent, and it is a person's action (`apply_finding_fix`,
  agent manage permission). It goes through `update_agent`, so the agent gets
  a new revision in its history with the approval's reason, and undoing it is
  restoring the previous revision.
- **Every fix is checked before it can be applied.** A check runs one of the
  agent's evals (the one with most cases, unless the person picks another) as
  an experiment with two variants, `current` and `with fix`, two trials per
  case by default. A fix can be applied only when the check found no case it
  broke and no regression verdict.
- **A fix applies only to the instructions it was checked against.** The check
  records a digest of the agent's instructions; applying refuses when they
  changed since, and the card asks for another check.
- **A person starts the check.** It costs model calls, and findings often come
  from work that is already expensive, so the Inspector never spends on checks
  by itself.
- **Suspected abuse never gets a fix.** Helping a misused agent work better is
  the opposite of what that finding is for; the Inspector is not shown the
  agent's instructions for it.
- **What a fix may say.** The Inspector is told never to loosen a safety rule,
  grant access or hide the problem, and to give no fix when the problem is not
  the instructions. Additions over 1,500 characters are dropped, not cut.

## Choices and their limits

- Only instruction additions for now. Tool, model and schedule changes need
  their own checks and come with self-healing.
- The finding is not added to the dataset by the check: the dataset belongs to
  its keepers. The check therefore proves the fix does no harm on the cases
  they keep and reports any case it fixed; it does not prove the fix stops the
  finding from recurring. "Add to dataset" from a session closes that gap.
- In the check, the addition goes into the eval target's `system_prompt`,
  which runs before the agent's own instructions; applying appends it after
  them. The agent reads the same text alongside its instructions either way.
- An agent with no eval cases cannot have fixes checked, so they cannot be
  applied: the card says to add cases first (the Eval Builder proposes them).

## Where it lives

- Inspector prompt and parsing: `crates/server/src/domains/health_issues/inspector.rs`
- Fix commands, check judging: `crates/server/src/domains/health_issues/fixes.rs`
- Storage: `crates/server/src/storage/finding_fixes.rs`, migration `finding_fixes`
- Finding page card: `apps/ui/src/components/evals/suggested-fix.tsx`
