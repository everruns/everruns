---
type: Specification
title: "Public Skills"
description: "Single source of truth for the public Everruns agent skills, who ships copies, how copies stay in sync, and the getting-started journeys they serve."
tags:
  - everruns
  - project
  - skills
---
# Public Skills

## Source of truth

`skills/` in this repository is the only place a public Everruns skill is
authored. Every other copy is generated from it and is never edited by hand.
A skill that teaches an agent to work with Everruns from outside (install it,
connect to it, author agents, call it) belongs here, not in a consumer.

## Consumers

| Consumer | What it carries | How the copy is made | Drift guard |
|---|---|---|---|
| This server | `everruns`, `everruns-cli`, `everruns-mcp`, `everruns-agent-authoring`, served over MCP `skills/get` | embedded at build time, [`crates/server/build.rs`](../../crates/server/build.rs) | build fails on a missing skill |
| [everruns/plugins](https://github.com/everruns/plugins) | the catalog's `sharedSkills`, shipped inside the coding-agent plugin | its `scripts/sync-shared-skills.mjs --bump` | its CI compares with a fresh clone of main, on push and weekly |
| [everruns/landing](https://github.com/everruns/landing) | every skill listed in its `scripts/sync-agent-skills.mjs`, served at `everruns.com/.well-known/agent-skills/<name>/SKILL.md` and listed in `llms.txt` | its `scripts/sync-agent-skills.mjs` | its CI compares with a fresh clone of main, on push and weekly |

Each consumer owns the list of skills it carries, so adding a skill here does
not publish it anywhere until a consumer lists it. Plugin-only workflow skills
(`everruns-run-agent`, `everruns-ship-agent`, ...) are authored in plugins
because nothing else ships them.

## Propagation

[`.github/workflows/skills-sync.yml`](../../.github/workflows/skills-sync.yml)
runs on every push to main that touches `skills/`. For each consumer,
[`scripts/sync-public-skills.sh`](../../scripts/sync-public-skills.sh) runs that
consumer's own copy step and commits as the owner of the sync token. Each
consumer lands it the way it lands everything else: plugins commits straight to
main and has no branch rules, so the copy is pushed to main; landing's main
ruleset requires a pull request (and its Cloudflare preview builds from one), so
the copy opens a pull request that merges itself once landing's CI is green.
The token is `SKILLS_SYNC_GITHUB_TOKEN` in Doppler: a fine-grained token with
contents and pull-request write on plugins and landing, owned by a maintainer so
commits carry a human identity. Without "Allow auto-merge" in landing's
settings the PR stays open and landing's drift check keeps reporting it.

## Rules for skill text

- **Instructions live only in skills.** A docs page, README or `llms.txt` shows
  a prompt as an example a person gives their agent, in a `text` fence, and
  never addresses the agent reading the page. An agent follows a skill because
  the person's prompt named it or their host loaded it; imperative text aimed
  at a reading agent elsewhere looks like prompt injection, and careful agents
  flag it. People and agents see the same content.
- **The person's own install first.** The demo instance (`app.everruns.com`) is
  a demo, not a hosted service: skills use it only when the person asks, and
  never as a fallback for an unreachable install.
- **Ask before changing anything outside the conversation**, keep secrets in
  the person's shell or in Everruns pages, and end with a check the person can
  see.

## Getting-started journeys

Getting-started pages lead with one example prompt that links the skill doing
the work, then the manual steps. The prompt carries the full skill URL, so the
agent learns where the skill lives and nothing else needs maintaining.

| Journey | Skill |
|---|---|
| Run it on this machine, a server, or Kubernetes; upgrade; remove | `everruns-install` |
| Connect a coding agent to a deployment | `everruns-plugin` (landing), plugins README |
| Build an agent into a Rust application | `everruns-framework` |
| Create and revise agents on a deployment | `everruns-agent-authoring` |
| Drive a deployment over HTTP | `everruns-api` |
| Contribute or fork | `AGENTS.md`, `CONTRIBUTING.md` (no skill) |
