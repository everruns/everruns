# Shipping Specification

## Abstract

This specification defines goal-oriented shipping for Everruns. Shipping should complete the requested goal, gather convincing evidence, create a mergeable PR, and merge only after CI is green.

The canonical agent workflow lives in [`.claude/skills/ship/SKILL.md`](../.claude/skills/ship/SKILL.md). That skill is intentionally user-invocable so shipping can be requested directly as `/ship`.

## Design Goals

Shipping work should optimize for these outcomes:

1. Reach the requested goal, not just perform rituals around it.
2. Match validation depth to the actual risk surface.
3. Keep affected artifacts and release metadata in sync.
4. Merge only from a safe branch state with green CI.

## Ownership Boundary

- This spec owns the shipping intent, constraints, and success bar.
- The skill owns the execution workflow, heuristics, and example commands.
- Other specs remain the source of truth for their domains. Shipping should update them when the change affects those domains rather than re-describe them here.

Relevant references:
- [`specs/release-process.md`](./release-process.md)
- [`specs/threat-model.md`](./threat-model.md)
- [`specs/code-organization.md`](./code-organization.md)
- [`specs/commands.md`](./commands.md)
- [`specs/skills-registry.md`](./skills-registry.md)

## Required Outcomes

**Every shipped change MUST satisfy ALL of these outcomes. These are mandatory requirements, not optional suggestions. Do not skip or weaken any requirement.**

1. Safe branch state: no shipping from `main` or `master`; working tree clean before final push; rebased onto the latest `origin/main` before merge.
2. Goal achieved with evidence: the requested behavior is implemented and validated with proof that matches the risk.
3. Merge-ready code: touched code is reviewed for avoidable complexity and performance risk. A structured security review is performed against the relevant threat model categories from `specs/threat-model.md` (see the Security Review section in the ship skill). Issues found during review are addressed or explicitly blocked.
4. Synced artifacts: only the affected artifacts are updated, including specs, threat model, docs, OpenAPI, test cases, and agent instructions when relevant.
5. Smoke test impacted functionality: always smoke test the flows affected by the change end-to-end. This is mandatory, not conditional on risk assessment. Docs-only or config-only changes that do not affect runtime behavior may skip smoke testing with explicit justification.
6. Safe merge: the PR uses the repo template, CI is green, **every** review comment from all reviewers (including async bot reviewers and low-confidence suggestions) is explicitly analyzed, reasoned about, and resolved — either with a code change or a written explanation — after a final post-green sweep, and merge happens with squash only.

## Constraints

- Shipping is outcome-oriented, not a mandatory linear checklist.
- Validation should start with the smallest high-signal proof and deepen only when risk or weak signals require it.
- Bug fixes should prefer a failing test before the fix when practical, but the validation strategy may vary when a smaller or stronger proof exists.
- Docs-only or config-only changes may skip code tests if that choice is justified and the relevant docs or build checks were run.
- Security review is mandatory for all code, configuration, and infrastructure changes. The review must identify relevant threat model categories, check the diff against them, and document findings. Perceived low risk does not justify skipping the review.
- Every review comment must be explicitly addressed before merge — including low-confidence suggestions, nits, and bot comments. For each comment, the agent must analyze the concern, reason about whether a change is warranted, and either apply a fix or reply with a clear explanation. Dismissing or ignoring comments without reasoning is not permitted.
- Auto-merge must not bypass the final review pass; shipping should give async reviewer bots time to comment after the last push and after CI turns green, then re-check before merge.
- If a blocker cannot be resolved safely by the agent alone, shipping must stop and report the blocker rather than guess.

## Reporting Standard

Shipping output should make it easy to evaluate readiness:

- what changed
- what evidence was gathered
- security review: which threat categories were checked, any findings, and how they were resolved
- what was skipped and why
- review comments: how each comment was addressed (code change or written reasoning)
- what blockers or residual risks remain
