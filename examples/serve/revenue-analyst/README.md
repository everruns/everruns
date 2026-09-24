# revenue-analyst (serve, experimental)

The full [serve](../../../crates/serve) project layout in one app. It runs
offline. The warehouse is an in-memory SQLite, and without a model gateway the
agent follows a scripted demo conversation.

```sh
cargo run -p serve-example-revenue-analyst              # dev server on :3000
cargo run -p serve-example-revenue-analyst -- eval      # 2 passed
cargo run -p serve-example-revenue-analyst -- manifest  # the host contract
cargo run -p serve-example-revenue-analyst -- deploy    # what a host would provision
```

## A tour, in a second terminal

```sh
# 1. Ask. The agent loads the sql-style skill and runs a cheap, date-filtered query.
ID=$(curl -s localhost:3000/v1/sessions -H 'content-type: application/json' \
  -d '{"input":"What was revenue last week?"}' | jq -r .id)
curl -sN "localhost:3000/v1/sessions/$ID/events?follow=false" | grep '^event:'

# 2. Ask for everything. A query with no WHERE clause needs approval, so the
#    turn pauses. The dev console prints a ready-made curl to approve it:
curl -s localhost:3000/v1/sessions/$ID/messages -H 'content-type: application/json' \
  -d '{"input":"Now show me every order we have ever had."}'
curl -s localhost:3000/v1/sessions/$ID/approvals/<approval_id> \
  -H 'content-type: application/json' -d '{"decision":"approve"}'

# 3. Ask for a review. The analyst delegates to the reviewer subagent.
curl -s localhost:3000/v1/sessions/$ID/messages -H 'content-type: application/json' \
  -d '{"input":"Check the query with the reviewer"}'

# 4. Stop the server (Ctrl+C), start it again, and message the same $ID. The
#    session resumes from .serve/ (session.resumed on the stream). The offline
#    script starts over in the new process; the conversation history does not.

# 5. A Slack mention (without SLACK_BOT_TOKEN the reply is printed, not posted).
curl -s localhost:3000/v1/channels/slack -H 'content-type: application/json' \
  -d '{"type":"event_callback","event":{"type":"app_mention","channel":"C42","ts":"1.1","text":"<@U1> revenue?"}}'

# 6. Fire Monday's schedule now.
curl -s -X POST localhost:3000/dev/schedules/weekly
```

Offline, each session plays the same script: (1) a cheap query, (2) a full
scan that waits for approval, (3) a question for the reviewer. Set
`OPENROUTER_API_KEY` to get a real model instead; then the model decides what
to do.

## Layout

| File | Is |
|---|---|
| `serve.toml` | name, `sandbox.kind = "bashkit"`, deploy target |
| `agent/instructions.md` | the always-on prompt |
| `agent/skills/sql-style/SKILL.md` | a skill; no code needed |
| `src/agent.rs` | `#[agent] fn analyst()` and its offline script |
| `src/tools/run_sql.rs` | `#[tool(needs_approval = …)] async fn run_sql` |
| `src/connections/warehouse.rs` | `#[connection]`, a typed value tools reach via `cx.connection::<Warehouse>()` |
| `src/connections/linear.rs` | `#[connection]`, an MCP server (skipped until `LINEAR_TOKEN` is set) |
| `src/channels/slack.rs` | `#[channel]`, Slack Events API in and `chat.postMessage` out |
| `src/schedules/weekly.rs` | `#[schedule("0 9 * * MON")]`, which posts to Slack |
| `src/subagents/reviewer.rs` | `#[agent(sub)]`, exposed to the analyst as `ask_reviewer` |
| `evals/revenue.rs` | two `#[eval]`s |
