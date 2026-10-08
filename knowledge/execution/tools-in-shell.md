---
type: Decision
title: "Tools in Shell"
description: "Proposed tools_in_shell capability: every tool an agent has becomes a `tools` command in the Bashkit shell, taking and returning JSON, plus saved scripts an agent keeps and a schedule can run without an LLM turn."
tags:
  - everruns
  - execution
  - bashkit
  - mcp
---

# Tools in Shell

> Status: **Proposed 2026-10-08, not implemented.** Inspired by
> [Executor](https://executor.sh) (one `execute` tool over a searchable tool
> catalog) and its v2 "apps" (agent-written tools that run on a schedule).
> Choices made with the requester are recorded under [Decisions](#decisions).

## Summary

An agent with the `tools_in_shell` capability reaches its tools through the
shell instead of as separate tool calls. Every tool it has, built-in, MCP or
integration, becomes a command:

```bash
tools --help                                   # sources: github, linear, slack (not loaded), ...
tools search "open pull requests"              # ranked matches, each with a line to run
tools github list-pulls --help                 # one tool's signature and input schema
tools github list-pulls '{"repo":"everruns/everruns","state":"open"}' \
  | jq '[.[] | select(.draft | not) | {number, title}]'
tools linear create-issue <<'JSON'
{"team": "EVE", "title": "Flaky test", "labels": ["ci"]}
JSON
```

What this changes for the person using the agent:

- **The prompt stops growing with every MCP server.** The model sees `bash`
  and a one-line list of sources, not hundreds of tool schemas.
- **Multi-service tasks take one or two steps instead of many.** The agent
  calls several tools in one script, filters with `jq`, and only the result it
  prints reaches the conversation.
- **Raw API payloads stay out of the context.** A 500-item list is filtered in
  the shell, not read by the model.
- **Agents can keep what they build.** A script that worked can be saved as a
  tool, called later like any other, and run on a schedule with no model in the
  loop.

## Why Bashkit

Bashkit is the main execution runtime, models write shell fluently, `jq` is
built in, and the shell already hosts a command surface: the `everruns`
command tree ([command tree](command-tree.md)) with bounded `--help` and
builtins that call registered tools
([`crates/integrations/src/bashkit/cli/`](../../crates/integrations/src/bashkit/cli/)).
The [Lua code mode](lua-execution.md) proved the pattern (one script, many
tool calls, a third fewer calls than bash on its eval) but would add a second
interpreter as the primary path. It stays an experiment and gets no further
investment; if `tools_in_shell` wins its eval, Lua code mode is removed.

## Decisions

### D1. One capability, `tools_in_shell`, per agent

Turned on per agent like any capability ("Tools in Shell"); it depends on
`bashkit_shell` and installs one command, `tools`. Turning it on does two
things:

1. **Installs the `tools` builtin**, rebuilt from the session's tool registry
   on every shell call, so it always reflects what the agent can call now.
2. **Moves eligible tools behind it.** A `ToolDefinitionHook` (the mechanism
   `lua_code_mode` uses) drops them from the model's direct tool list and adds
   the list of sources to the `bash` tool description. They stay in the
   executable registry, so the builtin can still run them.

#### Which tools go behind `tools`

The rule: a tool goes behind `tools` when calling it only does something and
returns data. It stays a direct tool when the model or the turn needs it as a
tool call. Concretely, these stay direct:

| Stays direct | Why |
|---|---|
| Turn-control tools: `ask_user`, `request_approval`/`record_approval`, client-side tools, subagent and handoff tools, `write_todos` | They pause or shape the turn, or run on the client; a shell command cannot. |
| Tools whose result the model must see as an image (browser, computer use) | Shell output is text. |
| The execution tools (`bash`, `lua`) and structured file editing | They are the way in, or the hot path where a structured call beats a heredoc. |
| Anything in the capability's `keep_visible` config | Per-agent override. |

Everything else goes behind: MCP tools, integration tools, search and fetch,
platform and data tools. Approval-gated tools also go behind (they ask, see
D7). The tool's owner marks the exceptions with a per-tool "stays direct" flag,
the same way owners mark `DeferrablePolicy::Never` for tool search today, so
new tools default to going behind and nobody maintains a central list.

#### Relationship to tool search

`tools_in_shell` replaces tool search for the tools it hides: they are not in
the model's tool list, so there is nothing for `tool_search` to defer, and
`tools search` inside the shell takes over discovery, using the same ranking.
The few tools that stay direct are normally under the deferral threshold, so
client-side tool search goes idle on its own. Provider-hosted tool search may
stay on for them; the two do not touch the same tools. Deferred MCP
placeholders (`mcp_<server>`) are hidden too and appear in the shell as
`(not loaded)` sources (D6).

### D2. Input is a JSON object; flags are a convenience

A tool already takes a JSON object checked against its schema, so `tools`
passes that object through rather than generating a command line per tool.
The model writes the same object it would send in a direct call, and there is
no per-tool parser to build or keep in sync. Accepted forms:

| Form | Example |
|---|---|
| JSON argument (canonical; shown in help and search) | `tools github get-pull '{"number":42}'` |
| JSON on stdin: heredoc, here-string or pipe | `jq -n '{number: 42}' \| tools github get-pull` |
| `key=value` pairs | `tools github get-pull number=42 repo=a/b` |
| `--key value` or `--key=value` | `tools github get-pull --number 42` |

For pairs and flags the value is read as JSON when it parses (`42`, `true`,
`["ci"]`) and as a string otherwise; kebab and snake case map to the same
field. Pairs and flags merge over a JSON argument or stdin object. The tool's
own schema check is the only validation: unknown or mistyped fields are
refused with the tool's signature in the error, never silently dropped.

The `everruns` command keeps its flag grammar; the two commands sit side by
side and do not share a parser.

### D3. Output is JSON on stdout, errors on stderr

Success: exit code 0 and the tool's result as JSON on stdout. Failure: a
non-zero exit code and `{"error":{"code","message","retryable"}}` on stderr,
so `set -e`, `||` and `jq` behave. Files and images are written to the
workspace and their path printed; binary data never goes to stdout.

### D4. Discovery is help plus search

`tools --help` lists sources, one line each. `tools <source> --help` lists its
tools. `tools <source> <tool> --help` prints a compact signature
(`get-pull({ number: integer, repo?: string })`), the description and the full
input schema. `tools search <words>` reuses the ranking of the
[`tool_search`](tool-search.md) tool and prints each match as a runnable line.
Names: an MCP tool `mcp_<server>__<tool>` is `tools <server> <tool>`; other
tools group under their capability; underscores become hyphens.

### D5. Every call is an ordinary tool call

Each command runs its tool through `ToolContext::nested_tool_policy`, as Lua
code mode and `spawn_background` do: pre-tool hooks, the schema check and
post-tool hooks run per call, and each call emits its own `tool.started` and
`tool.completed`, so the session timeline, audit and narration show every real
call, not one opaque shell call. A per-execution cap on calls (the forwarding
builtin's 50) stops runaway loops, and the error points at list-style tools.

### D6. All MCP servers, including the ones that appear later

MCP tools are already registry proxies, so every server is a source:

| Server | In the shell |
|---|---|
| Agent-attached and catalog servers | Listed from turn start; calls use the attachment's `actsAs`. |
| A person's own servers ([user MCP servers](../integrations/user-mcp-servers.md)) | Same, acting as that person; a missing grant is an error naming the server, and the usual Connect card applies. |
| Deferred servers (load on demand) | Listed as `(not loaded)`. The first call or search hit **loads it inside the same shell call** through the identity-scoped discovery cache and writes the usual `mcp_reveal:<server>` record. Today a reveal only takes effect on the next step; the builtin must not wait for that. |
| Added mid-session (ARD, chat-only servers) | Present from the next shell call, because the builtin is rebuilt from the registry each time. |

The one new host piece is loading a deferred server's tools on demand from
inside a tool call.

### D7. Approvals: ask once per script, before it runs

Hard approval ([tool approval](tool-approval.md)) and soft approval
([soft approval](soft-approval.md)) both keep working; the shell changes when
the question is asked, not who answers it.

**Before the script runs.** Bashkit's `analyze()` parses a script without
running it and reports each command with its literal arguments. The `bash`
tool uses it to find every `tools <source> <tool>` call in the script. If any
of those tools needs approval, the person gets **one approval card for the
whole script**, listing the actions in plain words ("Create issue in Linear",
"Delete branch `fix-x` in everruns/everruns"), with the script attached. On
approval the script runs and those exact calls go through; on refusal it does
not run. One card per script, not one per call, is the point: a script that
creates three issues asks once.

**When analysis cannot see the call.** Analysis is advisory: a tool name built
in a variable, `eval`, or a loop over computed names is reported as unknown.
Then the call is caught when it happens. Bashkit can suspend a live shell on a
builtin call while the host waits (host calls), so the script pauses, the
person sees the card, and the same script continues on approval. That pause
lives in the worker's memory: if it outlasts a short window or the worker
restarts, the script ends with `needs approval: requested`, the turn parks
durably on the request as it does today, and after approval the model runs the
command again. Calls that already succeeded in the turn are remembered by tool
and input, so the rerun does not repeat them. A refusal fails that call with
`{"error":{"code":"denied"}}`; nothing already done is undone.

**Soft approval.** Soft approval stays prompt guidance: the agent batches safe
work and calls `request_approval` before critical actions. With the shell, the
natural unit is the script: the agent asks before running a script with
critical steps, describing them. To make that description accurate,
`tools plan` takes a script and prints what it would call and how risky each
call is, from the same analysis, without running anything. Soft approval never
skips a hard-approval card; the two layers stay separate.

**How risky is a call.** Deterministic first, from what the tool says about
itself: MCP annotations (`readOnlyHint`, `destructiveHint`, `openWorldHint`)
already map to `ToolHints`, and a tool with `destructiveHint: true` asks first
unless an admin overrides it. Analysis adds shell-level facts (writes to files,
`rm`, opaque constructs). The [Decision Service](../operations/decisions-service.md)
fills only the gap: a tool that declares no hints (common for third-party MCP
servers) is rated once when it first appears, "does this tool change or delete
anything outside the session?", and the answer is cached per tool and schema.
It is not called per script, and it can only make a tool more cautious: it never
removes an approval an admin or the tool itself asked for. It is off when the
deployment has no decision service configured.

**Unattended runs.** A scheduled script (D9) has no one to ask, so a call that
needs approval fails as `denied`.

### D8. Saved scripts: tools an agent writes and keeps

A **saved script** is a shell script an agent owns: a name, a one-line
description, an optional input schema, and the body. It is an agent-owned
resource like a [trigger](../runtime-resources/agent-triggers.md), managed
through the command catalog (so it appears in the `everruns` tree and the API)
and recorded in the generic change history
([change reasons](change-reasons-and-manager-context.md)).

- **Calling.** `tools scripts <name> '{...}'` runs it in a nested shell over
  the same session filesystem with the same `tools` command. Its input arrives
  on stdin as JSON; its stdout is the result.
- **Saving.** `tools scripts save` from the shell, gated by the capability's
  `manage_scripts` config (off by default), the same use/manage split
  [user MCP servers](../integrations/user-mcp-servers.md) use.
- **Permissions.** A saved script runs with the **caller's** tools and
  identity, never its author's. It cannot reach anything the caller could not
  reach directly.
- **Data.** Scripts keep data with the existing `session_sqldb` capability; no
  storage of their own.

### D9. Schedules run saved scripts without a model

A schedule or webhook [trigger](../runtime-resources/agent-triggers.md) can
target a saved script instead of a message. The durable scheduler fires, the
script runs as the agent's identity in the trigger's session, and the run is
recorded there as events, with no LLM call. Approval-gated calls fail as
`denied` because no one is there to answer. A failed run can optionally wake
the agent with a message describing the failure.

## Not adopted

From Executor v2: approval rules written in code, deployment versions and
rollback, multi-step resumable workflows for scripts, per-app web UI, and
publishing or copying apps between people. Storage per app is covered by
`session_sqldb`.

## Success bar

- An eval slice with 50 to 300 MCP tools across several servers, where tasks
  need two or three of them, compares an agent with `tools_in_shell` against
  `auto_tool_search`: task success at least equal, fewer tokens and steps per
  task, wrong-tool calls reported. The capability stays opt-in until it passes.
- Every nested call is visible in the session events and audit log.
- No tool is reachable through `tools` that the agent could not call directly.

## Risks

- **Quoting.** JSON inside single quotes breaks on a value containing a single
  quote; stdin (heredoc or `jq -n ... |`) is the answer, and the capability's
  prompt says so.
- **Search quality.** With tools hidden, weak descriptions on third-party MCP
  tools show up as wrong-tool calls; the eval includes such servers.
- **Threat model.** Implementation adds `TM-TOOL` entries for nested calls from
  the shell, one-time approval grants, and unattended script runs, alongside
  the existing `TM-BASH` coverage ([threat model](../security/threat-model.md)).

## Plan

1. `tools` builtin and `tools_in_shell` capability (D1 to D5), behind a feature
   flag, with the eval slice.
2. MCP on-demand loading inside a shell call (D6).
3. Approvals from the shell (D7): per-script cards from analysis, host-call pauses,
   rerun fallback, then hint ratings from the decision service.
4. Saved scripts (D8).
5. Trigger-run of saved scripts (D9).
6. If the eval favors it, remove Lua code mode.
