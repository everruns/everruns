---
type: Specification
title: "Self-healing"
description: "What Everruns does on its own when a finding opens, how an organization chooses it per kind of action, and how every action is recorded, announced and undone."
tags:
  - everruns
  - evaluation
  - detection
---

# Self-healing

Status: implemented (Evals rebuild, step 11). Builds on
[suggested fixes](suggested-fixes.md) and the runaway detection in
[background limits](../operations/background-limits.md).

## What it is

An organization decides, per kind of action, what Everruns does when a
finding opens:

| Mode | Meaning |
|---|---|
| `auto` | take the action at once, record it on the finding, tell the people the finding notifies |
| `suggest` | offer the action on the finding for a person to take |
| `off` | neither |

The policy is `self_healing` on the organization (Settings → Limits, "When a
finding opens"; `PATCH /v1/orgs/{id}` with `self_healing`). Two kinds today:

- `runaway_schedules` (default `auto`): pause a schedule whose runs ended the
  same way 24 times in a row, instead of letting it run to the 48-run stop.
- `agent_fixes` (default `suggest`): whether the Inspector proposes changes to
  an agent's instructions. `off` stops the proposals; `auto` is refused.

## Contracts

- **Only protective, reversible actions are automatic.** A pause stops work
  the org's own detectors judged useless, and resuming restores the schedule
  exactly, with its repeated-outcome count started over.
- **A person approves every change to an agent.** Applying an instruction
  change automatically would need checks that start and spend on their own,
  and an automatic revert when live quality drops. Until that is designed, the
  policy rejects `agent_fixes: auto` rather than accept a value it cannot
  honour.
- **`off` never lifts a hard limit.** The 48-run stop and every budget stop
  apply whatever the policy says.
- **Every action is on the record.** Automatic or not, each action is a row in
  `finding_actions` (who took it, or nobody for automatic, and its undo),
  listed on the finding with Undo. One action of a kind per finding episode.
- **Announced like the finding.** An automatic pause opens the runaway finding
  with "Schedule paused: it kept repeating" and notifies the schedule's owner
  and the agent's authors.
- **Platform-side abuse actions are not part of the policy.** Suspension on
  classifier evidence always waits for a person on the platform side.

## Choices and their limits

- Stopping a looping session and capping a session's spend are protective too,
  but stopping a turn needs the runtime's cancel path, which background sweeps
  do not hold; they are left for a later step.
- The pause uses the existing runaway stop reason, so a paused schedule reads
  the same in the session as one the 48-run limit stopped; the finding says
  which.

## Where it lives

- Policy, scheduler hook, commands: `crates/server/src/domains/health_issues/self_healing.rs`
- Scheduler call: `crates/server/src/background/session_scheduler/mod.rs`
- Storage: `crates/server/src/storage/finding_actions.rs`, migration `self_healing`
- Finding page card: `apps/ui/src/components/evals/finding-actions.tsx`
- Settings card: `apps/ui/src/components/organizations/self-healing-settings.tsx`
