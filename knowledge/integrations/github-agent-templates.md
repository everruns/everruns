---
type: Decision
title: "GitHub review and security agent templates"
description: "Why the PR Reviewer and Security Scanner are agent examples with a guided setup, how they avoid repeating themselves, and which of their limits the tools enforce rather than the prompt."
tags:
  - everruns
  - integrations
  - github
  - agents
---

# GitHub review and security agent templates

## Abstract

Two guided templates give parity with hosted code-review and security-scan
products on any model: **PR Reviewer** (`pr-reviewer`) reviews every pull
request with inline comments, and **Security Scanner** (`security-scanner`)
scans a repository in a sandbox on a schedule and files deduplicated findings.
Both are built only from parts that already existed: the agent's own GitHub App
([github-apps.md](github-apps.md)), GitHub and schedule triggers
([agent-triggers.md](../runtime-resources/agent-triggers.md)), the `github`
capability, and the Daytona sandbox.

## Decisions

- **Templates are agent examples with a setup.** They are adopted through the
  same `POST /v1/agents/import?from-example=` path and listed by the same
  `/v1/agent-examples` endpoint; the only addition is an optional `setup` the UI
  walks through after import (connect GitHub, pick a repository, choose
  settings, create the trigger). There is no parallel catalogue. They live in
  `crates/server/src/agent_templates.rs` because `seed.rs` is at its size limit.
- **The trigger default is a normal create-trigger body.** It carries a
  `${repository}` placeholder (distinct from the `{{…}}` the trigger renders at
  fire time) that the client fills in before posting to the triggers API, so
  trigger validation has one home.
- **Settings are enforced by the capability, not the prompt.** A setting maps to
  one `github` capability config key. Repository content is untrusted and could
  argue a model out of a prompt rule, but not out of a tool it was never given:
  without `allow_pull_requests` there is no `create_github_pull_request` tool,
  and with `private_issues_only` the issue tool refuses public repositories.
- **Fix pull requests are off by default, and need a second, deliberate step.**
  Agent Apps request contents read only, so even with the setting on, pushing a
  branch fails until the owner grants Contents write on GitHub. The
  least-privilege default stays the default App.
- **Repeat suppression is deterministic.** The reviewer runs per pull request
  (`per_thread`), so a push continues the same session, but the tool does not
  rely on the model remembering: inline comments carry a finding key, keys the
  App already posted are dropped, and a second review of the same head commit
  is a no-op. Issues are keyed by a fingerprint: open ones are updated, closed
  ones stay closed. Only bot-authored text counts, so a marker pasted by a
  person suppresses nothing.
- **The reviewer cannot approve.** An approving agent could satisfy branch
  protection on the strength of attacker-written input.
- **Untrusted input stays out of the trigger message.** The reviewer's message
  names the repository, number, action and URL but not the title; the agent
  reads attacker-written text through its tools, framed as data by its prompt.

## Where it lives

- Templates: `crates/server/src/agent_templates.rs`; API shape:
  `crates/server/src/api/agent_examples.rs`
- Tools and capability config: `crates/integrations/src/github/` (`reviews.rs`,
  `issues.rs`, `fix_pull_requests.rs`, `GitHubConfig` in `lib.rs`)
- Setup UI: `apps/ui/src/components/agents/agent-template-setup.tsx`
- How-to: `docs/how-to/set-up-review-and-security-agents.md`
- Threats: TM-GHAPP-008 to TM-GHAPP-011 in
  [threat-model.md](../security/threat-model.md)
