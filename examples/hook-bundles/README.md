# Hook bundles

Ready-to-paste configurations for the
[`user_hooks`](https://docs.everruns.com/capabilities/user-hooks/)
capability.

| File | Shape | What it does |
|---|---|---|
| [`block-rm-rf.json`](./block-rm-rf.json) | `user_hooks` config | Block `rm -rf /...` calls on the `bash` tool. |
| [`format-on-edit.json`](./format-on-edit.json) | `user_hooks` config | Run `cargo fmt` and `prettier` after each `edit_file`. |
| [`session-bootstrap.json`](./session-bootstrap.json) | `user_hooks` config | Drop a sentinel file at session start. |
| [`audit-every-tool.json`](./audit-every-tool.json) | `user_hooks` config | Append every tool call to `/workspace/.audit.log`. |
| [`block-secret-prompt.json`](./block-secret-prompt.json) | `user_hooks` config | Block a user prompt that pastes a private key (`user_prompt_submit`). |
| [`turn-end-log.json`](./turn-end-log.json) | `user_hooks` config | Append one line per completed turn to `/workspace/.turn-log` (`turn_end`). |

## Using a hook config

Add the `user_hooks` capability to your agent with the corresponding
JSON as its `config` field:

```bash
curl -X POST http://localhost:9301/v1/agents \
  -H "Content-Type: application/json" \
  -d '{
    "name": "safer-coder",
    "display_name": "Safer Coder",
    "system_prompt": "You are a helpful coding agent.",
    "capabilities": [
      "session_file_system",
      "bashkit_shell",
      {
        "ref": "user_hooks",
        "config": '"$(cat block-rm-rf.json)"'
      }
    ]
  }'
```

Risk: enabling `user_hooks` requires an admin role on the agent (same
gate as `bashkit_shell` and `web_fetch`).

## Muting a capability-contributed hook

When a built-in capability ships a hook bundle, users can mute
individual entries via the `disabled_contributions` field on a sibling
`user_hooks` capability config:

```json
{
  "ref": "user_hooks",
  "config": { "disabled_contributions": ["some_capability:audit_tool_calls"] }
}
```

## Sharing bundles across an organization

Declarative-capability hook bundles (one POST to register, many agents
to reuse) are on the roadmap, see the "Deferred" note in
[`knowledge/runtime-resources/user-hooks.md`](../../knowledge/runtime-resources/user-hooks.md). Until then, paste
the JSON from this directory into each agent's `user_hooks` config.

## See also

- [`docs/capabilities/user-hooks.md`](../../docs/capabilities/user-hooks.md), capability docs
- [`knowledge/runtime-resources/user-hooks.md`](../../knowledge/runtime-resources/user-hooks.md), full contract

## Live end-to-end demo: guarded bash

The `guarded-bash-demo` seed agent pairs the `block-rm-rf.json` bundle
with a scripted `LlmSim` driver. To watch a `pre_tool_use` hook refuse a
destructive command end-to-end without any LLM API keys, run the server
with the bundled scripted driver:

```bash
LLMSIM_DEMO=guarded \
DEV_MODE=true DEPLOYMENT_GRADE=dev AUTH_MODE=none \
HTTP_ADDR=127.0.0.1:9301 SERVER_GRPC_BIND_ADDR=127.0.0.1:9302 \
SECRETS_ENCRYPTION_KEY="kek-v1:8B3uCQ4Znx45hl5nB+PKVriRrj/KtEVM+wBZ2VGa9vY=" \
OTEL_SDK_DISABLED=true \
cargo run -p everruns-server
```

The scripted model first attempts `rm -rf /`, then a safe `ls`. With the
`pre_tool_use` block hook attached, the first tool call is refused before
it ever reaches the sandbox and the second succeeds, the agent observes
the difference in tool results.

The `LLMSIM_DEMO=guarded` switch is opt-in. Without it, `LlmSim` returns
its default canned response and never calls any tool.

> The former `LLMSIM_DEMO=auditor` walkthrough (Cloud Cost & Security
> Auditor over fake AWS tools) was retired with EVE-875: the fake demo
> capabilities moved to `everruns-test-support` and are no longer
> registered by product binaries.
