---
title: Tools in Shell
description: Call an agent's tools, MCP servers included, from the Bashkit shell with one tools command, so one script can chain several calls and return only what matters.
appliesTo: [framework, platform, cloud]
---

| | |
|---|---|
| **ID** | `tools_in_shell` |
| **Category** | Execution |
| **Risk** | High, assignment requires an org **Admin** |
| **Depends on** | [Bashkit Shell](/capabilities/bashkit-shell/) |
| **Rollout** | `FEATURE_TOOLS_IN_SHELL` grade (adoption by default) |

An agent with many tools spends most of its prompt on tool schemas and most of
its turns on one call at a time. Tools in Shell moves most tools out of the
model's direct tool list and into one shell command, `tools`, inside the
[Bashkit Shell](/capabilities/bashkit-shell/). One script can call several
tools, filter the results with `jq`, and print only what the model needs:

```bash
tools github list-pulls '{"repo":"acme/api","state":"open"}' \
  | jq -r '.[] | select(.draft | not) | .number' \
  | while read n; do
      tools github get-pull "repo=acme/api" "number=$n" | jq -c '{number, title, mergeable}'
    done
```

## The `tools` command

| Command | What it does |
|---|---|
| `tools --help` | Lists MCP servers (with their tool counts) and the other tools |
| `tools <server> --help` | Lists one MCP server's tools |
| `tools <tool> --help`, `tools <server> <tool> --help` | Shows the tool's input schema |
| `tools search <words>` | Finds tools by name and description |
| `tools <server> <tool> '{...}'` | Calls an MCP tool |
| `tools <tool> '{...}'` | Calls any other tool |

MCP tools are grouped by server (`mcp_github__list_pulls` is
`tools github list-pulls`); every other tool is a top-level command
(`web_fetch` is `tools web-fetch`). Hyphens and underscores are interchangeable.
The list is rebuilt on every shell call, so an MCP server added during a
session shows up on the next one.

**Input** is the same JSON object the tool takes as a direct call. It can come
from stdin (`jq -n '{repo: "a/b"}' | tools github list-pulls`, or `-`), from a
JSON argument, or from `key=value` and `--key value` pairs, which merge over the
JSON. `--flag` alone is `true`. A value is read as JSON unless the tool's schema
says the key is a string, so `number=7` is a number and `repo=42` stays text.

**Output** is the tool's result on stdout: a text result as is, anything else
as JSON. Images a tool returns are saved under `tool-images/` in the working
directory, and stderr names the files.

**Errors** are one JSON line on stderr with exit code 1, so a script can branch
on them:

```json
{"error":{"code":"needs_approval","message":"...","retryable":false}}
```

| Code | Meaning |
|---|---|
| `unknown_command` | No such server or tool |
| `invalid_input` | The input is not a JSON object or does not match the tool's schema |
| `tool_error` | The tool ran and failed |
| `denied` | A guardrail or hook blocked the call |
| `needs_approval` | The call needs a person's approval; the script stops here (see [Approvals](#approvals)) |
| `stopped` | An earlier call stopped the script, so this one did not run |
| `connection_required` | The tool or its MCP server needs a connection the person has not made |
| `call_limit` | More than 50 tool calls in one shell call |
| `unavailable` | The command cannot run here, or a server could not load yet (`retryable` says whether a later step can) |

## Which tools move into the shell

A tool moves behind `tools` when calling it only does something and returns
data: MCP tools, integration tools, search and fetch, platform and data tools.
These stay direct tool calls:

- the `bash` and `lua` tools themselves;
- tools that pause or shape the turn: `ask_user`, approvals, `spawn_agent`,
  session tasks, `write_todos`, and client-side tools;
- tools whose result the model must see as an image, such as screenshots,
  `read_file`, `computer` and `browser`;
- structured file editing (`write_file`, `edit_file`);
- tools listed in the `keep_visible` config.

MCP servers set to "Load tools on demand" move behind `tools` too. `tools
--help` lists them as `(not loaded)`; the first command that names one, such as
`tools docs --help` or `tools docs search-docs q=refunds`, or a `tools search`
that matches the server, loads its tools inside the same shell call. From the
next step on the server is listed like any other.

Every call from a script goes through the same checks as a direct call: the
turn's guardrails and hooks run before it, the tool's schema is checked, and
the post-tool hooks see the result before the script does.

## Approvals

Approval-gated and destructive tools are reachable from the shell too, and
[Tool Approval](/capabilities/tool-approval/) judges each call from a script
the same way it judges a direct call. Most scripts never ask: reads, searches,
and tools a person already allowed "always" just run.

When a call written out in full in the script, such as `tools github
delete-branch branch=fix-x`, needs a person's answer, the script does not start
at all: the `bash` result says so (`"before_start": true` under `stopped`,
`done` is empty), the person is asked about that call, and after they answer
the agent can run the same script again. Calls whose input is built while the
script runs, or that read their input from stdin, are checked as they run
instead.

To see what a script would do before running it, pass it to `tools plan`. It
runs nothing and prints each `tools` call it can see, with its input and risk:

```bash
tools plan <<'EOF'
tools github list-pulls state=open
tools github delete-branch branch=fix-x
EOF
```

```json
{
  "calls": [
    {"tool": "tools github list-pulls", "input": {"state": "open"}, "risk": "read_only", "where": "script"},
    {"tool": "tools github delete-branch", "input": {"branch": "fix-x"}, "risk": "needs_approval", "where": "script"}
  ],
  "complete": true
}
```

`risk` is `read_only`, `changes` (runs without asking), `needs_approval`,
`blocked`, or `checked_at_run_time` when the input is built while the script
runs. `complete` is `false` when the script hides calls from reading, for
example with `eval`. The agent uses it to describe a script accurately before
asking a person about it.

When a call needs a person's answer while the script runs, it is not made and
the script stops there. Nothing after it runs, and the script is never resumed or run again,
because part of its work is already done and may not be safe to repeat. The
`bash` result reports what happened, and the person is asked about the stopped
call in the same step:

```json
{
  "exit_code": 1,
  "success": false,
  "tools": {
    "done": [
      {"tool": "tools linear create-issue", "input": {"title": "Flaky test"},
       "ok": true, "result": {"id": "EVE-1201"}}
    ],
    "read_only_calls": 3,
    "stopped": {
      "reason": "needs_approval",
      "approval": "requested",
      "call": {"tool": "tools github delete-branch", "input": {"branch": "fix-x"}}
    },
    "not_reached": "the stopped call and everything after it; write a new script for the rest"
  }
}
```

`done` lists every call that may have changed something; read-only calls are
counted. After the person answers, the agent writes a new script for what is
left. A one-off approval covers that exact call (the tool and its input) once,
so the new script can make it. The same report appears when a script stops at
the call limit, or exits with an error after changing something.

## In the session timeline

The model sees one `bash` call. Each tool the script called through `tools` is
also recorded in the session as a
[`tool.nested_call`](/event-reference/) event under that `bash` call, with its
command, a short preview of its input, whether it completed, failed, was
refused or stopped the script for approval, and how long it ran. The chat shows
these as rows under the shell call. They never reach the model.

## Saved scripts

An agent can keep a script it wrote and run it again later as one command:

```bash
cat <<'EOF' | tools scripts save triage-prs --description 'Label new pull requests.' \
  --input-schema '{"type":"object","properties":{"repo":{"type":"string"}},"required":["repo"]}'
repo=$(jq -r .repo)
tools github list-pulls repo="$repo" state=open | jq -r '.[].number' | while read -r n; do
  tools github add-labels repo="$repo" number="$n" labels='["triage"]'
done
EOF

tools scripts                      # list the agent's saved scripts
tools scripts triage-prs repo=a/b  # run one
```

- The input is checked against the script's schema and arrives on stdin as
  JSON; what the script prints is its result.
- A saved script runs with the tools and identity of the agent turn that runs
  it, and its `tools` calls follow the same rules: approvals, guardrails, and
  the 50-call limit of the shell call it runs in. A call that needs approval
  stops the caller's script too.
- Its shell has the shell builtins and `tools`; `curl` and the `everruns`
  command are not available inside it.
- Saved scripts can call each other, up to four deep.
- Saving needs the `manage_scripts` setting. Every agent with the capability
  can run its saved scripts. Scripts are also managed through the
  `/v1/agents/{agent_id}/scripts` API and the `everruns agents scripts` command.
- A schedule or webhook [trigger](/features/agent-triggers/#run-a-saved-script)
  can run a saved script without a model call.

## Tool search

Tools in Shell replaces tool search. When both are enabled, the tool search
capabilities (`tool_search`, `auto_tool_search`, `openai_tool_search`,
`claude_tool_search`) contribute nothing, and
[Agent Checks](/features/agent-checks/) report the redundant capability as a
`capabilities.superseded` suggestion.

## Configuration

```json
{
  "capabilities": [
    { "ref": "tools_in_shell", "config": { "keep_visible": ["web_fetch"] } }
  ]
}
```

| Field | Type | Default | Meaning |
|---|---|---|---|
| `keep_visible` | string array | `[]` | Tools the model can still call directly. They stay callable from the shell too. |
| `manage_scripts` | boolean | `false` | Let the agent save scripts with `tools scripts save`. |
