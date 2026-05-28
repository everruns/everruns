# Hook bundles

Ready-to-paste configurations for the [`user_hooks`](https://docs.everruns.com/capabilities/user-hooks/)
capability and JSON bodies for [declarative
capabilities](https://docs.everruns.com/capabilities/) that ship reusable
hook bundles.

| File | Shape | What it does |
|---|---|---|
| [`block-rm-rf.json`](./block-rm-rf.json) | `user_hooks` config | Block `rm -rf /...` calls on the `bash` tool. |
| [`format-on-edit.json`](./format-on-edit.json) | `user_hooks` config | Run `cargo fmt` and `prettier` after each `edit_file`. |
| [`session-bootstrap.json`](./session-bootstrap.json) | `user_hooks` config | Drop a sentinel file at session start. |
| [`audit-every-tool.json`](./audit-every-tool.json) | `user_hooks` config | Append every tool call to `/workspace/.audit.log`. |
| [`rust-quality-pack.declarative.json`](./rust-quality-pack.declarative.json) | Declarative capability body | Shareable bundle: fmt + clippy after Rust edits. Risk: High. |
| [`security-pack.declarative.json`](./security-pack.declarative.json) | Declarative capability body | Shareable bundle: deny dangerous bash patterns. Risk: High. |

## Using a hook config

Add the `user_hooks` capability to your agent with the corresponding
JSON as its `config` field:

```bash
curl -X POST http://localhost:9300/api/v1/agents \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Safer Coder",
    "system_prompt": "You are a helpful coding agent.",
    "capabilities": [
      "session_file_system",
      "virtual_bash",
      {
        "ref": "user_hooks",
        "config": '"$(cat block-rm-rf.json)"'
      }
    ]
  }'
```

Risk: enabling `user_hooks` requires an admin role on the agent (same
gate as `virtual_bash` and `web_fetch`).

## Using a declarative bundle

Register the bundle once per org, then reference it on any agent:

```bash
# 1. Register the bundle (admin only — High risk because it carries hooks).
curl -X POST http://localhost:9300/api/v1/capabilities \
  -H "Content-Type: application/json" \
  -d @rust-quality-pack.declarative.json

# 2. Enable on an agent.
curl -X POST http://localhost:9300/api/v1/agents \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Rust Agent",
    "capabilities": ["session_file_system", "virtual_bash", "declarative:rust_quality_pack"]
  }'
```

A user can mute any contributed hook from the bundle via the
`disabled_contributions` field on the user-facing `user_hooks` capability:

```json
{
  "ref": "user_hooks",
  "config": { "disabled_contributions": ["rust_quality_pack:clippy_after_edit"] }
}
```

## See also

- [`docs/capabilities/user-hooks.md`](../../docs/capabilities/user-hooks.md) — capability docs
- [`specs/user-hooks.md`](../../specs/user-hooks.md) — full contract
