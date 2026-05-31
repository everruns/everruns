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
payload through environment variables (`$EVERRUNS_HOOK_PAYLOAD_JSON` plus a
copy at `$EVERRUNS_HOOK_PAYLOAD_PATH` on the session VFS) and reads a
structured decision back from stdout. (Delivery is via env vars rather than
process stdin because the `virtual_bash` interpreter runs the script
in-process and exposes no process stdin.) Hooks can run silently (logging
only), mutate the inputs they observe, or block the action outright.

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

Six events. Two (`pre_tool_use`, `user_prompt_submit`) can block; the rest
are advisory-only. **`pre_tool_use` and `post_tool_use` fire today**; the
other four events ship their schema/validation here but their runtime
wire-in lands in follow-up changes (see "What's not yet wired" below).

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

The bash executor is modeled after Claude Code hooks with one departure:
the payload is delivered via env vars (and a session-VFS file) rather than
stdin, because the bashkit interpreter doesn't expose process-level stdin
to user scripts.

### Input — env vars + VFS file

Every hook invocation sets these env vars:

| Var | Meaning |
|---|---|
| `EVERRUNS_HOOK_PAYLOAD_JSON` | Full payload, JSON-encoded as one string. Read with `jq -n 'env.EVERRUNS_HOOK_PAYLOAD_JSON \| fromjson'`. |
| `EVERRUNS_HOOK_PAYLOAD_PATH` | Path on the session VFS to a file containing the same JSON. Cleaned up after the hook returns. Use for `cat "$EVERRUNS_HOOK_PAYLOAD_PATH" \| jq` workflows. |
| `EVERRUNS_HOOK_EVENT` | One of `session_start`, `user_prompt_submit`, `pre_tool_use`, `post_tool_use`, `turn_end`, `session_end`. |
| `EVERRUNS_HOOK_ID` | Stable hook id (`{capability_id}:{name}` or `user:{name}`). |
| `EVERRUNS_HOOK_SESSION_ID` | Current session id. |
| `EVERRUNS_HOOK_TURN_ID` | Set when a turn is in flight. |
| `EVERRUNS_HOOK_TOOL_NAME` | Set for `pre_tool_use` / `post_tool_use`. |
| `EVERRUNS_HOOK_TOOL_CALL_ID` | Set for `pre_tool_use` / `post_tool_use`. |

Payload envelope:

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

Any built-in capability can contribute hook specs to your agent by
overriding `Capability::user_hooks_with_config` and returning a list of
`UserHookSpec`s. The `cloud-cost-security-auditor` seed agent is a live
example: its `user_hooks` capability config (in
[`crates/server/src/seed.rs`](https://github.com/everruns/everruns/blob/main/crates/server/src/seed.rs))
ships an audit-log `post_tool_use` hook that fires every time the agent
runs an AWS tool, with no extra setup.

Capability-contributed hooks ride the trust gate of enabling the
contributing capability, so admin assignment rules still apply.
Operators can mute any individual contribution via
`disabled_contributions` on a sibling `user_hooks` capability config.

> Declarative-capability hook bundles (one POST to register, many agents
> to reuse) are on the roadmap and are not yet wired into the declarative
> capability schema. See
> [`specs/user-hooks.md`](https://github.com/everruns/everruns/blob/main/specs/user-hooks.md)
> for the deferred contract.

See [`examples/hook-bundles/`](https://github.com/everruns/everruns/tree/main/examples/hook-bundles)
for ready-to-paste user-config bundle JSON.

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
              "args_jsonpath": "$.commands",
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

Today, hook decisions and errors are recorded in the server logs via
`tracing`: blocks, ignored post-hook blocks, mutations, `on_error`
outcomes, and `disabled_contributions` mutings are logged with the
resolved `hook_id` and the `tool_call_id`.

**Planned (deferred):** structured `hook.invoked` / `hook.completed` /
`hook.blocked` / `hook.warning` events emitted through the same
observability pipeline as tool events (Braintrust, OTel). These are not
emitted yet — see the spec's observability section.

## What's not yet wired

`pre_tool_use` and `post_tool_use` **fire today**: a pre-hook can block or
mutate a tool call before it runs, and a post-hook can mutate the tool
result after it runs.

The remaining events (`session_start`, `user_prompt_submit`, `turn_end`,
`session_end`) ship their *contract, types, executor, and validation* here
but their runtime firing points land in follow-up changes so reviewers can
audit each one against the relevant execution atom in isolation. Configuring
those events is a validated no-op until their wire-in ships. Track progress
against
[`specs/user-hooks.md`](https://github.com/everruns/everruns/blob/main/specs/user-hooks.md).

## See also

- [`specs/user-hooks.md`](https://github.com/everruns/everruns/blob/main/specs/user-hooks.md) — full contract
- [`specs/capabilities.md`](https://github.com/everruns/everruns/blob/main/specs/capabilities.md) — capability framework
- [`specs/threat-model.md`](https://github.com/everruns/everruns/blob/main/specs/threat-model.md) — TM-HOOK entries
- [Virtual Bash](/capabilities/virtual-bash/) — the sandbox that runs hook commands
