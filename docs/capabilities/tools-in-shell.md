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
| `needs_approval` | The call needs a person's approval; the model should ask for it directly |
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
- for now, approval-gated and destructive tools;
- tools listed in the `keep_visible` config.

MCP servers set to "Load tools on demand" move behind `tools` too. `tools
--help` lists them as `(not loaded)`; the first command that names one, such as
`tools docs --help` or `tools docs search-docs q=refunds`, or a `tools search`
that matches the server, loads its tools inside the same shell call. From the
next step on the server is listed like any other.

Every call from a script goes through the same checks as a direct call: the
turn's guardrails and hooks run before it, the tool's schema is checked, and
the post-tool hooks see the result before the script does.

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
