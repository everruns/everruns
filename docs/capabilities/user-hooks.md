---
title: User Hooks
description: Run user-authored shell commands at lifecycle and tool events. Block, mutate, or audit agent actions from outside the model.
sidebar:
  order: 96
---

| | |
|---|---|
| **ID** | `user_hooks` |
| **Category** | Automation |
| **Features** | None |
| **Dependencies** | None |
| **Risk** | High |
| **Spec** | [specs/user-hooks.md](https://github.com/everruns/everruns/blob/main/specs/user-hooks.md) |

User Hooks let you inject shell commands at six well-defined points in the
agent execution lifecycle. The runtime hands each hook a structured JSON
payload on stdin and reads a structured decision back from stdout. Hooks
can run silently (logging only), mutate the inputs they observe, or block
the action outright.

This is the same pattern you'll recognize from Claude Code hooks, Git
hooks, and pre/post-tool middleware in agent SDKs — applied as a
first-class capability so any agent can adopt it, any hook bundle can be
shared across an organization, and every invocation lands in the audit log.

## When to enable

- **Security gates** — block any `bash` call matching `rm -rf /` before
  the sandbox sees it.
- **Format-on-write** — run `cargo fmt` or `prettier` after every
  `edit_file`.
- **Audit / observability** — POST every tool call to your SIEM.
- **Project bootstrapping** — at `session_start`, clone a repo, install
  deps, or seed a workspace.
- **CI-style validation** — at `turn_end`, run tests and surface
  failures back to the model on the next turn.

## Risk

High. The capability accepts arbitrary shell commands from config. Even
though the commands run inside the session's `virtual_bash` sandbox
(no host filesystem, no host network beyond the session's egress policy),
the assignment gate is admin-only — anyone who can configure this
capability can run arbitrary code in the session sandbox on every agent
action.

## Events

Six events. Three can block; three are advisory-only.

| Event | Fires at | Can block? | Mutation surface |
|---|---|---|---|
| `session_start` | New session created | no | none |
| `user_prompt_submit` | User message accepted, before reason | yes | user message text |
| `pre_tool_use` | Each tool call, before execution | yes | `ToolCall.arguments` |
| `post_tool_use` | Each tool call, after execution | no | `ToolResult` |
| `turn_end` | Turn finishes | no | none |
| `session_end` | Session close/archive | no | none |

See [`specs/user-hooks.md`](https://github.com/everruns/everruns/blob/main/specs/user-hooks.md)
for the per-event JSON payload shape and full block / mutate semantics.

## Configuration

The capability accepts two top-level fields:

```json
{
  "capabilities": [
    {
      "ref": "user_hooks",
      "config": {
        "hooks": [ /* UserHookSpec entries */ ],
        "disabled_contributions": [ /* HookId strings to mute */ ]
      }
    }
  ]
}
```

### `UserHookSpec`

```jsonc
{
  "id": "fmt_after_edit",                    // optional; defaults to "{event}_{idx}"
  "event": "post_tool_use",
  "matcher": { "tool_name": "edit_file" },   // tool events only
  "executor": {
    "type": "bash",
    "command": "scripts/fmt.sh",
    "env": { "FMT_PROFILE": "ci" }
  },
  "timeout_ms": 5000,                        // 100..30_000
  "on_error": "warn",                        // "block" | "allow" | "warn"
  "description": "Run formatter after edit_file"
}
```

#### Matcher

The `matcher` block applies only to `pre_tool_use` and `post_tool_use`.
Setting it on lifecycle events is rejected at validation time.

| Field | Meaning |
|---|---|
| `tool_name` | Exact tool name match |
| `tool_name_glob` | Restricted glob: `a\|b\|c` alternation or trailing `*` |
| `args_jsonpath` | Dot-path into `ToolCall.arguments` (e.g. `$.command`) |
| `match_regex` | Fires when extracted value matches this regex |
| `deny_regex` | Inverse — fires when extracted value matches this regex |

`match_regex` and `deny_regex` are mutually exclusive. Regex flavor is the
Rust `regex` crate (no look-around, no backreferences).

#### `on_error`

What to do when the executor itself fails (timeout, non-JSON output,
sandbox error):

- `block` — treat as `{"decision": "block", "reason": "hook failed"}`.
  Use for security-critical hooks.
- `allow` — log + continue.
- `warn` — log + emit `hook.warning` event + continue. **Default.**

## Bash hook contract

The bash executor mirrors Claude Code so existing scripts port with
minimal changes.

### Input — stdin

```json
{
  "event": "pre_tool_use",
  "hook_id": "user:guard_rm",
  "session_id": "ses_…",
  "turn_id": "trn_…",
  "org_id": "org_…",
  "agent_id": "agt_…",
  "ts": "2026-05-28T12:34:56.789Z",
  "data": {
    "tool_name": "bash",
    "tool_call_id": "call_…",
    "arguments": { "command": "ls -la" }
  }
}
```

### Output — stdout

Three accepted shapes, tried in this order:

1. **Empty stdout** — exit 0 = allow; non-zero = block (stderr surfaced
   as the block reason). This is the Git-hook escape hatch.
2. **JSON decision** — stdout starts with `{`:

   ```json
   {
     "decision": "allow" | "mutate" | "block",
     "reason": "string shown in audit/UI",
     "user_message": "string surfaced to the user when blocking",
     "patch": { /* event-specific mutation */ }
   }
   ```

3. **Anything else** → executor error → `on_error` policy applies.

### Limits

- **Timeout** — configurable per hook; default 5 s, max 30 s.
- **Output size** — 64 KiB total (stdout + stderr).
- **Sandbox** — runs through `virtual_bash` against the session VFS. No
  host shell. Inherits the session's egress policy.
- **stderr** — captured into the audit log; never shown to the model
  unless `decision == "block"` with no `reason`.

## Composition

Hooks chain in capability-declaration order, then array order within each
capability. The first `block` decision wins; mutations from earlier hooks
survive even if a later hook blocks.

Capabilities other than `user_hooks` can ship hook bundles — see [Hook
bundles from other capabilities](#hook-bundles-from-other-capabilities).
To mute a bundled hook, list its `HookId` under
`disabled_contributions`:

```json
{
  "capabilities": [
    { "ref": "rust_quality_pack" },
    { "ref": "user_hooks", "config": {
      "disabled_contributions": ["rust_quality_pack:fmt_after_edit"]
    } }
  ]
}
```

`HookId` format:

- Capability contribution: `{capability_id}:{name}`
- User config: `user:{name}`

## Hook bundles from other capabilities

Any capability — built-in, MCP, declarative — can contribute hook
specs to your agent. Bundled hooks ride the trust gate of enabling that
capability, so admin assignment rules still apply. The user-facing
config surface is identical to the one above; the difference is the
source field, which the runtime stamps automatically.

Declarative capabilities are the easiest way to share a bundle across
an organization:

```bash
curl -X POST http://localhost:9300/api/v1/capabilities \
  -H "Content-Type: application/json" \
  -d @scripts/rust-quality.json
```

See [`examples/hook-bundles/`](https://github.com/everruns/everruns/tree/main/examples/hook-bundles)
for ready-to-paste bundle JSON.

## Examples

### Block `rm -rf` from the bash tool

```json
{
  "capabilities": [
    {
      "ref": "virtual_bash"
    },
    {
      "ref": "user_hooks",
      "config": {
        "hooks": [
          {
            "id": "guard_rm",
            "event": "pre_tool_use",
            "matcher": {
              "tool_name": "bash",
              "args_jsonpath": "$.command",
              "deny_regex": "(?:^|;|&&|\\|)\\s*rm\\s+-rf\\b"
            },
            "executor": {
              "type": "bash",
              "command": "echo '{\"decision\":\"block\",\"reason\":\"rm -rf is not allowed\",\"user_message\":\"That command is blocked by policy.\"}'"
            },
            "on_error": "block",
            "description": "Reject destructive rm invocations"
          }
        ]
      }
    }
  ]
}
```

### Format Rust files after every edit

```json
{
  "capabilities": [
    { "ref": "session_file_system" },
    { "ref": "virtual_bash" },
    {
      "ref": "user_hooks",
      "config": {
        "hooks": [
          {
            "id": "fmt_rs",
            "event": "post_tool_use",
            "matcher": {
              "tool_name": "edit_file",
              "args_jsonpath": "$.path",
              "match_regex": "\\.rs$"
            },
            "executor": {
              "type": "bash",
              "command": "cargo fmt --check 2>&1 || cargo fmt"
            },
            "on_error": "warn"
          }
        ]
      }
    }
  ]
}
```

### Seed a workspace on session start

```json
{
  "capabilities": [
    { "ref": "session_file_system" },
    { "ref": "virtual_bash" },
    {
      "ref": "user_hooks",
      "config": {
        "hooks": [
          {
            "id": "init",
            "event": "session_start",
            "executor": {
              "type": "bash",
              "command": "echo 'session bootstrapped' > /workspace/.bootstrap && echo '{}'"
            },
            "on_error": "warn",
            "description": "Drop a sentinel file the agent can read"
          }
        ]
      }
    }
  ]
}
```

## Observability

Every hook execution emits four audit events:

- `hook.invoked` — fired when matcher passes.
- `hook.completed` — `{ hook_id, decision, duration_ms, stdout_bytes, exit_code }`.
- `hook.blocked` — only when `decision == block`.
- `hook.warning` — only when `on_error == warn` fires.

These flow through the same observability pipeline as tool events
(Braintrust, OTel).

## What's not yet wired

This capability ships the *contract, types, executor, validation, and
audit surface* end-to-end. The runtime firing points for each event land
in follow-up changes so reviewers can audit each one against the
relevant execution atom in isolation. The capability is safe to enable
today — it accepts and validates config — but hooks will not actually
fire until each event's wire-in PR ships. Track progress against
[`specs/user-hooks.md`](https://github.com/everruns/everruns/blob/main/specs/user-hooks.md).

## See also

- [`specs/user-hooks.md`](https://github.com/everruns/everruns/blob/main/specs/user-hooks.md) — full contract
- [`specs/capabilities.md`](https://github.com/everruns/everruns/blob/main/specs/capabilities.md) — capability framework
- [`specs/threat-model.md`](https://github.com/everruns/everruns/blob/main/specs/threat-model.md) — TM-HOOK entries
- [Virtual Bash](/capabilities/virtual-bash/) — the sandbox that runs hook commands
