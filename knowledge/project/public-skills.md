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

There is one public skill, [`everruns`](../../skills/everruns/SKILL.md). Its
`SKILL.md` is a short map; every task guide (Framework, install, CLI, MCP, API,
SDKs, building, running, debugging, evaluating and shipping agents, operations,
agents as files) is a file under `references/`. Decision (2026-11-17): one skill
instead of fourteen, following the one-product-skill-per-repository pattern
(`npx skills add <owner>/<repo>` installs it without flags). One name to
install, one folder to copy, and the agent reads only the guides a task needs.
The cost is triggering: a host picks a skill by its description, so that one
description must name every job; the plugin evals check it.

Contributor skills live in `.agents/skills/` and carry `metadata.internal:
true`, so skill installers hide them.

## Consumers

| Consumer | How the copy is made | Drift guard |
|---|---|---|
| `npx skills add everruns/everruns` | reads `skills/everruns/` from main | none needed |
| This server: MCP `skills/get` and `skill://everruns/...`, Platform Chat mounts | embedded at build time, [`crates/server/build.rs`](../../crates/server/build.rs), [`crates/capabilities/build.rs`](../../crates/capabilities/build.rs) | build fails on a missing skill |
| The CLI: `everruns agent-skill install/show` | embedded at build time, [`crates/cli/build.rs`](../../crates/cli/build.rs); a crates.io build has no workspace and points at `npx skills add` | built from the same tree |
| [everruns/plugins](https://github.com/everruns/plugins) | its `scripts/sync-shared-skills.mjs --bump` | its CI compares with a fresh clone of main, on push and weekly |
| [everruns/landing](https://github.com/everruns/landing), `everruns.com/.well-known/agent-skills/everruns/` (with `references/` beside `SKILL.md`) | its `scripts/sync-agent-skills.mjs` | its CI compares with a fresh clone of main, on push and weekly |

The server and CLI copies match their release; npx, the plugin and landing
follow main. The skill therefore tells agents to trust the installed version's
`--help`, `discover` and docs.rs over remembered flags.

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

Getting-started pages lead with one example prompt, then the manual steps. The
prompt links `https://everruns.com/.well-known/agent-skills/everruns/SKILL.md`
so it works without installing anything, and the box notes that with the skill
installed (`npx skills add everruns/everruns`) the link can go. The skill's map
sends each journey to its guide: install, framework, build-agent, api, mcp.
Contributing or forking stays with `AGENTS.md` and `CONTRIBUTING.md` (no skill).
