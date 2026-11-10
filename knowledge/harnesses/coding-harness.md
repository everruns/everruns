---
type: Specification
title: "Coding Harness"
description: "Provider-neutral coding behavior across one or several GitHub repositories, based on Yolop, whose execution target is selected by an Agent Sandbox Template binding."
tags:
  - everruns
  - harnesses
  - environments
---
# Coding Harness

`coding` is an adoptable, provider-neutral harness example, tagged `beta`. It
inherits Sandbox Worker and adds coding behavior taken from
[Yolop](https://github.com/everruns/yolop): the working habits in the prompt,
code navigation ([Code Navigation](code-navigation.md)), and a set of
repositories per session. It requires a full sandbox but does not select a
compute provider.

## Decisions

- **The Everruns loop works against a plain sandbox** (user, 2026-11-09). No
  Yolop binary runs in the sandbox and Yolop's ACP mode is not used, because
  both put agent logic next to the code. The sandbox holds only developer
  tooling: git and language servers.
- **Everruns cannot depend on the `yolop` crate.** Yolop pins the `everruns-*`
  crates at one published version, so pulling it in would bring a second copy
  of the core types. Shared code moves into Everruns and Yolop uses it from
  there: code navigation lives in `everruns-coding`.
- **Multi-repository from the first version.** One repository is a set with one
  entry; there is no separate single-repository layout.
- **`coding` stays a harness**, marked beta with a tag the harness list shows.
- **Language servers are on by default** and start on first use.
- **The prompt names no tools and no providers.** It is Yolop's workflow in
  those terms; the environment and the repository list are derived at runtime.

What was deliberately not brought over from Yolop: git worktrees (a session's
sandbox is already the isolation), checkpoint and rewind through git snapshots
(sandboxes have their own checkpoints), commit attribution trailers (the
agent's GitHub App is the author), and everything that is the terminal product
(TUI, config, skills manager, editor bridge).

## Repositories

The `repositories` capability (`crates/integrations/src/github/repositories.rs`)
is step 6, "Sandbox repositories", of the [GitHub model](../integrations/github-model.md).

- **Set.** A list of `owner/repo` with a starting branch and `read` or `write`
  access, in the capability's config. The agent's config holds its usual
  repositories; a session created with its own `repositories` config uses that
  set instead (session config replaces agent config, as for every capability).
  The agent may add a repository mid-session; what it may do with an added one
  is the `add_repositories` setting (`off`, `read` by default, `write`).
- **Layout.** `/workspace/<name>`, or `/workspace/<owner>-<name>` when the name
  is taken by another repository. `/workspace` is not a repository.
- **No credential in the sandbox.** Each clone, fetch and push is a tool call
  that mints a token for that operation and hands it over for one git command.
  This is stricter than the "credential helper in the sandbox" of the first
  design: a standing helper would let any shell command push. The cost is that
  `git fetch` and `git push` typed into the shell do not work for private
  repositories, so the tools `repo_sync` and `repo_push` exist, and clones are
  shallow instead of blobless (a blobless clone fetches file contents later,
  from the shell, and would fail).
- **Access is config, not sandbox state.** `repo_push` authorizes by repository
  name against the config, mints the token for that repository, and pushes to
  the URL built from that name. The checkout's `origin` is the agent's to edit
  and decides nothing.
- **One work branch per session**, `<branch_prefix><session short id>`, the
  same in every repository. Writable repositories start on it. `repo_push`
  pushes only that branch and never forces.
- **Change set.** `repo_status` reports, per repository, the branch,
  uncommitted files, commits ahead of the base branch and commits not pushed.
  It is read from the sandbox on each call; nothing is kept in the worker.
- **Instructions.** A repository's `AGENTS.md` is returned with its first
  checkout, as data in the tool result.
- **Pull requests.** One per repository, opened with
  `create_github_pull_request` from the `github` capability, which the harness
  enables.

Token admission and the accepted exposure are TM-GITHUB-009 in the
[threat model](../security/threat-model.md).

## Verified, and not yet

Run on a local stack against a real Daytona sandbox with Muse 1.3 Contributor
(2026-11-09): the harness boots, `repo_status` reports a missing GitHub App
installation in plain words, and the repository map, structural search and
language-server tools (TypeScript) work on a checkout in the sandbox. That run
found that Daytona starts commands under zsh, so the sandbox workspace now runs
every script under `sh`.

Not verified against GitHub: checkout, fetch and push with an installation
token. The repository tools are covered by tests that run real git against
local remotes. The first deployment with a GitHub App connected to an agent
must run that leg (two writable repositories, one read-only, a push and a
refused push) before the harness loses its beta tag.

## Not built yet

- Secrets other than GitHub (package registries, API keys) for full sandboxes.
  Bound secrets work in the Bashkit shell only; the plan is to apply them at
  the egress proxy.
- Installing language servers into sandbox images. `lsp` names the missing
  server and the agent can install it with the shell.
- UI: a repository picker when starting a session and a Changes view per
  repository.
- Moving Yolop's patch-style edit tool and tool argument validation into
  Everruns.

## Tools on every target

The Agent owns a Sandbox policy with named template bindings. A Session selects one binding
and receives the same model-facing tools on every supported target:
`bash`, `read_file`, `write_file`, `edit_file`, `glob`, and `grep`. The resolved
specification is pinned to the Session's logical Sandbox.

- A policy that only allows `vfs` is rejected by Sandbox Worker; use Bashkit
  Worker for Bashkit-only coding agents.
- A `managed` Daytona profile replaces both with `session_sandbox`, so shell and
  file tools address one remote `/workspace` rather than two filesystems.
- The harness prompt names neither providers nor provider-specific tools. The
  Sandbox preamble is derived from the resolved target and containment.

The legacy `coding-daytona`, `coding-container`, and
`coding-session-sandbox` names remain reconciliation-only. Existing org rows
are preserved as editable custom harnesses, but they are no longer offered as
examples.

See `crates/server/src/harnesses/coding.rs` for the definition and
[Sandbox Templates and Execution Targets](sandbox-templates.md) for profile selection and
runtime semantics.
