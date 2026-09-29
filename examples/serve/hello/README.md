# hello (serve, experimental)

The smallest [serve](../../../crates/serve) app: one agent, one tool, one
eval. It runs offline; without a model gateway, the agent follows a scripted
simulator.

```sh
cargo run -p serve-example-hello              # dev server on :3000
cargo run -p serve-example-hello -- eval      # 1 passed
cargo run -p serve-example-hello -- manifest
```

```sh
ID=$(curl -s localhost:3000/v1/sessions -H 'content-type: application/json' -d '{}' | jq -r .id)
curl -s localhost:3000/v1/sessions/$ID/messages -H 'content-type: application/json' \
  -d '{"message":{"content":[{"type":"text","text":"Roll a d20"}]}}'
curl -N "localhost:3000/v1/sessions/$ID/sse?after_sequence=0"
```

| File | Is |
|---|---|
| `agent/instructions.md` | the always-on prompt (hot-reloads in `dev`) |
| `src/agent.rs` | `#[agent] fn assistant()` |
| `src/tools/roll_dice.rs` | `#[tool] async fn roll_dice(cx, sides)` |
| `evals/dice.rs` | `#[eval] async fn rolls_when_asked(t)` |

Set `OPENROUTER_API_KEY` to run the agent on a real model.
