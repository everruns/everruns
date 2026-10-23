---
type: Decision
title: "Tools in Shell"
description: "Implemented tools_in_shell capability: eligible tools are callable through the Bashkit shell, with CLI-only and hybrid native-plus-CLI modes."
tags:
  - everruns
  - execution
  - bashkit
  - mcp
---

# Tools in Shell

> Status: **Implemented** in
> `crates/integrations/src/bashkit/tools_in_shell/`. Inspired by
> [Executor](https://executor.sh) (one `execute` tool over a searchable tool
> catalog) and its v2 "apps" (agent-written tools that run on a schedule).
> Choices made with the requester are recorded under [Decisions](#decisions).

## Summary

An agent with the `tools_in_shell` capability can reach eligible built-in, MCP,
and integration operations through the shell. In the default `cli` mode those
operations use the shell instead of separate model calls; `hybrid` keeps both
routes available:

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

- **Native schemas can stay bounded.** In CLI mode eligible operations are
  represented through `bash`; hybrid mode also keeps them available natively.
  Hybrid alone leaves those schemas eager; configure bounded `tool_search`
  explicitly to defer optional native schemas. The inventory is discovered
  with `tools --help`, not copied into the static Bash tool description.
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
`bashkit_shell` and installs one command, `tools`. `mode` defaults to `cli`;
`mode: "hybrid"` also exposes eligible operations as native tools. Turning it
on does two things:

1. **Installs the `tools` builtin**, rebuilt from the session's tool registry
   on every shell call, so it always reflects what the agent can call now.
2. **Adds the configured CLI route for eligible tools.** In `cli` mode those
   operations are removed from the model's direct tool list. In `hybrid` mode
   they remain callable through both routes. Hybrid mode alone keeps native
   schemas eager; enabling `tool_search` with `bounded: true` separately defers
   optional schemas under its limits. Owner-required `Never` schemas remain
   eager. The live source list is discoverable through `tools --help`; it is
   not copied into the static Bash tool description. Operations remain in the
   same executable registry.

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

Eligible operations use the configured CLI route. Client-side, turn-control,
image-returning, execution, and owner-constrained tools retain required native
access. `keep_visible` keeps an eligible tool on both routes in CLI mode. In
hybrid mode eligible tools are already on both routes with native schemas
eager; bounded `tool_search` must be explicitly enabled to defer optional
schemas. Owner-required `DeferrablePolicy::Never` schemas remain eager.

#### Relationship to tool search

The [native and CLI surface plan](tool-surfaces.md) implements per-operation
routes. CLI mode supersedes generic and hosted tool-search capabilities because
eligible operations are absent from the model's native list. Hybrid mode
allows native search capabilities alongside the CLI route. Hybrid mode alone
keeps native schemas eager; bounded native discovery requires
`tool_search` configured with `bounded: true`.

`tools_in_shell` in CLI mode supersedes tool search; an agent should not have
both unless it uses hybrid mode:

- **In CLI mode, `tools_in_shell` wins.** Eligible tools are absent from the
  model's native list, and `tools search` provides CLI discovery. In hybrid
  mode, an explicitly configured bounded `tool_search` can reveal optional
  native schemas while shell search discovers the same authorized registry.
  Without it, native schemas stay eager. Deferred MCP placeholders
  (`mcp_<server>`) are represented in the shell as `(not loaded)` sources (D6).

- **The agent checks report it.** A built-in deterministic rule in
  [Agent Checks](../evaluation/agent-checks.md), `capabilities.superseded`,
  flags an agent that has `tools_in_shell` together with a tool search
  capability (which some presets add by default) as a `suggestion` in the
  `cost` category: "Tool Search does nothing while Tools in Shell is on;
  remove it." Like every agent check it is advisory and never blocks saving.
- **Both follow one declaration.** A capability lists what it replaces in
  `Capability::supersedes`; capability collection skips those, and the agent
  check reads the same list, so neither hard-codes the tool search ids. The
  rule is generic rather than tool-search specific for the same reason.

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
input schema. `tools search <words>` uses the same server-aware index as bounded
native [`tool_search`](tool-search.md). Operation matches print runnable
commands; server-description-only matches are labeled as server refinements,
not operation matches, and point to `tools <server> --help`. Legacy native
search retains its existing matcher.
Names: an MCP tool `mcp_<server>__<tool>` is `tools <server> <tool>`; every
other tool is a top-level command (`tools web-fetch`), because registry tools
carry no capability attribution to group them by; underscores become hyphens.
`tools search` ranks the shell's current authorized catalog rather than calling
the `tool_search` tool. In CLI mode the generic search tool is superseded; in
hybrid mode both routes share the bounded index.

### D5. Every call is an ordinary tool call

Each command runs its tool through `ToolContext::nested_tool_policy`, as Lua
code mode and `spawn_background` do: pre-tool hooks, the schema check and
post-tool hooks run per call. The shell catalog, search, help and invocation
are restricted to the current turn's visible-tool allowlist. A loaded MCP
child retains the scope of its visible server placeholder, and the nested
policy checks scope again before dispatch. A nested call is not emitted as its own
`tool.started`/`tool.completed`: those events materialize as conversation
messages, and a call the model never made would break the tool-call pairing
the conversation history relies on. Each call is recorded instead as one
`tool.nested_call` event when it ends (completed, failed, refused, or held for
approval), naming the `bash` call it ran in, so the session timeline and the
chat show every call without the model seeing it. A per-execution cap on calls (the forwarding
builtin's 50) stops runaway loops, and the error points at list-style tools.

### D6. All MCP servers, including the ones that appear later

MCP tools are already registry proxies, so every server is a source:

| Server | In the shell |
|---|---|
| Agent-attached and catalog servers | Listed from turn start; calls use the attachment's `actsAs`. |
| A person's own servers ([user MCP servers](../integrations/user-mcp-servers.md)) | Same, acting as that person; a missing grant is an error naming the server, and the usual Connect card applies. |
| Deferred servers (load on demand) | Visible placeholders are listed as `(not loaded)`. Naming one directly loads it in the same shell call. One `tools search` may load up to eight deferred servers whose names/descriptions match; any remaining placeholders stay pending for a later query. Successful loads write the usual `mcp_reveal:<server>` record. |
| Added mid-session (ARD, chat-only servers) | Present from the next shell call, because the builtin is rebuilt from the registry each time. |

`McpToolInvoker::list_server_tools` lists through the connection and account a
call would use. Only placeholders in the current turn scope can be searched,
shown in help, or loaded; calls to returned children retain that parent scope
and are checked again at dispatch. One shell search loads at most eight matching
deferred servers, then searches the tools returned by those loads. Unselected
pending servers remain available to later queries. A server's tools are added
atomically only if the resulting shell catalog stays within 10,000 tools and
32 MiB of serialized definitions; on overflow the server remains pending and
none of its operations become visible. Each listing is not cached by the
in-call path; its reveal record moves that server onto the host's cached
discovery path from the next step. A server that needs a sign-in returns
`connection_required` and is not revealed; a host that cannot list mid-call
still writes the reveal and answers `unavailable` (retryable), so the tools
arrive on the next step.

Loaded children retain the authorized placeholder's server name and description
as search context. Missing or blank values fall back to its prefix and visible
description; operation risk hints and unrelated metadata remain intact. A
repeated description-only query therefore still identifies the loaded source
without listing it again or treating the description as an operation match.

### D7. Approvals: most scripts never ask; a risky call stops the script

Hard approval ([tool approval](tool-approval.md)) and soft approval
([soft approval](soft-approval.md)) both keep working. The model is a safe
mode, not a gate in front of every script:

- **Most scripts run with no approval at all.** Each call is judged on its own
  risk; reads, searches and the tools an admin or an earlier "always" answer
  already allowed just run.
- **A risky call is never made without a person's answer**, and it is caught
  as early as possible so the script does as little as possible first.
- **A script is never resumed or run again.** Part of its work may already be
  done and may not be safe to repeat, so a stopped script ends, reports what
  happened, and the agent writes a new script for what is left.

**How risky is a call.** Deterministic first, from what the tool says about
itself: MCP annotations (`readOnlyHint`, `destructiveHint`, `openWorldHint`)
already map to `ToolHints`, and a tool with `destructiveHint: true` needs
approval unless an admin overrides it. Existing approval answers count: an
"always" answer allows every call of that tool for the rest of the session, a
one-off answer allows one exact call. The
[Decision Service](../operations/decisions-service.md) fills only the gap: a
tool that declares no hints (common for third-party MCP servers) is rated once
when it first appears, "does this tool change or delete anything outside the
session?", and the answer is cached per tool and schema. It is not called per
script, and it can only make a tool more cautious: it never removes an approval
an admin or the tool itself asked for. Files the script writes stay in the
session workspace and need no approval. As built (`tools_in_shell/ratings.rs`): only a
tool with an `Auto` policy and none of the readonly, destructive, idempotent
or open-world hints is rated, by the context's decision service, lazily at its
first call or early-stop preview; a "yes" adds `destructive` to the definition
the gate judges, and the answer is cached process-wide per tool name and a hash
of its description and schema. An unavailable service or failed rating
changes nothing and is not cached. An approval raised only because of a rating
says so: its `risk` reads `rated_changes` instead of `destructive`, so the card
and `tools plan` can tell a person the tool declared nothing and merely looks
like it changes things (user, 2026-10-10: ratings must be visible, then
correctable per tool, then switchable per agent). The per-agent switch is the
capability's `rate_unlabeled_tools` config (on by default): the host marks an
off turn's tool context (`UnlabeledToolRatingsOff`) and every rating path (run
time, early-stop preview, `tools plan`) skips the rating, so the tool is judged
by its absent hints exactly as with no decision service configured.

**1. Analyse before running, to stop early.** Bashkit's `analyze()` parses the
script without running it and lists every `tools` call it can see. If one of
those calls needs approval, the script **does not start**: the result names the
call and its line, and the approval request for it is raised (step 3). Nothing
has run, so nothing is half done. Scripts with no risky call, which is most of
them, start straight away. Analysis is advisory: a tool name built at run time
or `eval` is not visible to it, and step 2 catches those. As built, a call is
previewed only when its command and every argument are literal, it runs in the
script itself (not a function body or a substitution), and it does not read
stdin; the preview asks the pre-tool chain without prompting or using up an
answer (`PreToolUseHook::preview`), so the request binds to the exact call and
the unchanged script can run again after the answer. A call in an untaken
branch can still hold the script; the cost is one question asked early.

**2. Check each call as it happens.** Every `tools` call is judged again when
it runs, with its real input. A call that needs approval and has none is **not
made**: the script is stopped at that point.

**3. The stop reports what happened and asks.** The `bash` result carries a
report in place of a normal exit, and the approval card for the stopped call
is raised in the same step:

```json
{
  "stopped": {
    "reason": "needs_approval",
    "line": 9,
    "call": {"tool": "github delete-branch", "input": {"branch": "fix-y"}},
    "approval": "requested"
  },
  "done": [
    {"tool": "linear create-issue", "input": {"title": "Flaky test"},
     "ok": true, "result": {"id": "EVE-1201"}},
    {"tool": "github delete-branch", "input": {"branch": "fix-x"},
     "ok": true}
  ],
  "not_reached": ["line 9 onwards"],
  "stdout": "...", "stderr": "...",
  "files_written": ["/workspace/report.json"]
}
```

`done` lists every call that changed something, with its outcome and a trimmed
result; read-only calls are counted, not listed. Nothing already done is
undone. The same report shape is used when a script ends on a call error, the
call cap or a time limit, so "what happened" always reads the same way.

**4. After the answer, the agent writes a new script.** The turn parks on the
request the normal durable way, so it survives restarts and waits as long as it
needs. When the person answers, the agent gets the answer with the report and
writes a new script for the remaining work, from the real state rather than by
repeating the old one. A one-off approval covers that exact call (tool and
input) once, so the new script can make it; an "always" answer covers the tool
for the session; a refusal means the agent finds another way or tells the
person. The old script is never resumed or rerun.

**Soft approval.** Soft approval stays prompt guidance: the agent batches safe
work and calls `request_approval` before critical actions. With the shell the
unit is the script: the agent asks before running a script with critical
steps. `tools plan` takes a script and prints the calls analysis can see, with
each one's risk, without running anything, so the agent can describe the
script accurately. As built (`tools_in_shell/plan.rs`), it reads the script
with the same analysis and asks the pre-tool chain the same `preview` question
as the early stop, so a call it marks `needs_approval` is the call that would
hold the script; a call whose input is built at run time is judged on the tool
alone, and `complete: false` flags dynamic commands or `eval`. Soft approval
never skips a hard approval; the two layers stay separate.

**Saved scripts and unattended runs.** A saved script (D8) follows the same
rules inside its body. A scheduled run (D9) has no one to answer, so a call that
needs approval stops it with the same report and the request recorded for
later, never waiting.

### D8. Saved scripts: tools an agent writes and keeps

A **saved script** is a shell script an agent owns: a name, a one-line
description, an optional input schema, and the body. It is an agent-owned
resource like a [trigger](../runtime-resources/agent-triggers.md), managed
through the command catalog (so it appears in the `everruns` tree and the API)
and recorded in the generic change history
([change reasons](change-reasons-and-manager-context.md)).

- **Resource.** The storage half exists as the
  [agent script](../runtime-resources/agent-scripts.md) resource (CRUD under
  `/v1/agents/{agent_id}/scripts`); the shell surface builds on it.
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

As built: the worker binds the session's agent's scripts to the turn
(`runtime::saved_scripts` in contracts, `grpc_saved_scripts` in the worker),
and a script runs in its own Bashkit shell over the same session files that
shares the caller's `tools` builtin, so its calls count against the same run
and call cap and a stop inside it stops the caller. That shell has no `curl`
or `everruns` tree, and scripts nest at most four deep.

### D9. Schedules run saved scripts without a model

A schedule or webhook [trigger](../runtime-resources/agent-triggers.md) can
target a saved script instead of a message. The durable scheduler fires, the
script runs as the agent's identity in the trigger's session, and the run is
recorded there as events, with no LLM call. A call that needs approval stops
the run (D7), because no one is there to answer. A failed or stopped run
records the D7 report and can optionally wake the agent with that report.

As built: the trigger's `script` setting marks its message with a reserved
metadata key that client metadata loses
(`runtime::saved_scripts::SCRIPT_RUN_METADATA_KEY`). For that turn the host
swaps the model's driver for one that answers the first reason with a single
`bash` call (`tools scripts <name>`, input on stdin) and the second with a fixed
line (`crates/core/src/host/script_run.rs`). The turn engine is unchanged, so the
call keeps its approval gate, events and transcript. Waking the agent hands the
second reason to the real model. Only schedule and webhook triggers take a
script (`crates/server/src/domains/agent_triggers/script_target.rs`). Input
strings are templates over the event context; a lone placeholder keeps the
value's JSON type, which is how a webhook payload reaches the script.

## Not adopted

From Executor v2: approval rules written in code, deployment versions and
rollback, multi-step resumable workflows for scripts, per-app web UI, and
publishing or copying apps between people. Storage per app is covered by
`session_sqldb`.

## Qualification status

- The canonical Rust/Mira synthetic qualification is complete; see
  [Large MCP Catalogs](large-mcp-catalogs.md#canonical-rust-mira-study) for its
  matrix and interpretation.
- Every nested call is visible in the session events and audit log.
- No tool is reachable through `tools` that the agent could not call directly.

## Discovery eval

Whether a model finds a tool it cannot see is measured, not assumed, by the
[Tools in Shell study](../../evals/tools-in-shell/README.md): a Framework agent
with the shipped capability over a small fake registry, offline apart from a
model key. Its cases cover finding a tool by need and by source, choosing
between two similar tools, passing a JSON object with an enum and an array,
and recovering from a tool name that does not exist. Shell calls are counted
the way the [command tree](command-tree.md)'s eval counts them (help reads,
rejected calls, real calls), with the same classifier, so `tools` help can be
tuned with the same loop as `everruns` help. It is a precondition for the
success bar above, not the bar itself: it shows a tool can be found in few
calls, not that a 50 to 300 tool slice beats tool search.

## Risks

- **Quoting.** JSON inside single quotes breaks on a value containing a single
  quote; stdin (heredoc or `jq -n ... |`) is the answer, and the capability's
  prompt says so.
- **Search quality.** With tools hidden, weak descriptions on third-party MCP
  tools show up as wrong-tool calls; the eval includes such servers.
- **Threat model.** Implementation adds `TM-TOOL` entries for nested calls from
  the shell, one-off approvals reused by a rebuilt script, and unattended script runs, alongside
  the existing `TM-BASH` coverage ([threat model](../security/threat-model.md)).

## Implementation status

1. The shell builtin, capability routing, MCP loading, approval handling,
   saved scripts, and trigger execution described above are implemented.
2. The Rust/Mira synthetic qualification is complete; see
   [Large MCP Catalogs](large-mcp-catalogs.md#canonical-rust-mira-study). The
   prior Python prototype is historical evidence only.
3. Removing Lua code mode remains a separate decision based on evaluation.
