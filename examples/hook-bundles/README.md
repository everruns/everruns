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

## Live end-to-end demo: Cloud Cost & Security Auditor

The `cloud-cost-security-auditor` seed agent ships with the
`audit-every-tool` bundle. To see the audit log get written end-to-end
without any LLM API keys, run the server with the bundled scripted
`LlmSim` driver:

```bash
LLMSIM_DEMO=auditor \
DEV_MODE=true DEPLOYMENT_GRADE=dev AUTH_MODE=none \
HTTP_ADDR=127.0.0.1:9301 SERVER_GRPC_BIND_ADDR=127.0.0.1:9302 \
SECRETS_ENCRYPTION_KEY="kek-v1:8B3uCQ4Znx45hl5nB+PKVriRrj/KtEVM+wBZ2VGa9vY=" \
OTEL_SDK_DISABLED=true \
cargo run -p everruns-server
```

Then:

```bash
API=http://127.0.0.1:9301/api/v1

# Enable the LlmSim model
LLMSIM=$(curl -s $API/llm-models | jq -r '.data[] | select(.model_id=="llmsim-default") | .id')
curl -s -X PATCH -H "Content-Type: application/json" "$API/llm-models/$LLMSIM" -d '{"enabled":true}'

# Import the auditor (carries the user_hooks bundle)
AGENT=$(curl -s -X POST -H "Content-Type: application/json" \
  "$API/agents/import?from-example=cloud-cost-security-auditor" -d '{}' \
  | jq -r '.self_url | split("/") | last')

# Start a session and send a kickoff message
SESSION=$(curl -s -X POST -H "Content-Type: application/json" "$API/sessions" \
  -d "{\"agent_id\":\"$AGENT\",\"model_id\":\"$LLMSIM\"}" | jq -r '.id')
curl -s -X POST -H "Content-Type: application/json" \
  "$API/sessions/$SESSION/messages" \
  -d '{"message":{"content":[{"type":"text","text":"Run the audit."}]}}' >/dev/null

# Read the hook-written audit log
sleep 2
curl -s "$API/sessions/$SESSION/fs/.audit.log" | jq -r .content
```

You should see two timestamped lines, one per tool call:

```
[2026-…Z] aws_list_ec2_instances:call_demo_ec2
[2026-…Z] aws_list_s3_buckets:call_demo_s3
```

The `LLMSIM_DEMO=auditor` switch is opt-in. Without it, `LlmSim` returns
its default canned response and never calls any tool — the audit log is
empty.
