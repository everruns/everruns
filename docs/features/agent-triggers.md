---
title: Agent Triggers
description: Run an agent proactively on a recurring schedule, choose session reuse, test it immediately, and inspect recent outcomes.
appliesTo: [platform, cloud]
---

Agent triggers let an Agent start work without a user message. A trigger belongs to one Agent, runs on that Agent's Harness, and sends a configured message when it fires.

| `trigger_type` | Fires when | Set up |
|---|---|---|
| `schedule` | A cron expression matches | This page |
| `webhook` | An external system calls the trigger's token-authenticated URL | API (`token`, optional `filter`, `rate_limit_per_minute`) |
| `github` | A GitHub event reaches the Agent's GitHub App, such as a pull request | [Summarize GitHub pull requests](/how-to/summarize-github-pull-requests/) |
| `mcp_event` | One of the Agent's MCP servers emits a subscribed event | [MCP Events](/features/mcp/) |

The rest of this page covers schedule triggers.

## Create a trigger in the UI

1. Open the agent.
2. Select the **Integrations** tab.
3. Select **Add trigger**.
4. Choose a preset or enter a 5-field cron expression (or a 7-field expression whose year is `*`), then enter an IANA timezone. In 5-field cron, weekdays count 0-6 from Sunday (7 is also Sunday), so `0 9 * * 1-5` runs Monday through Friday. A 7-field expression counts weekdays 1-7 from Sunday, so its Monday through Friday is `2-6` (or `MON-FRI`). The editor shows the schedule in human-readable form and previews the next eight runs. The default timezone is `UTC`.
5. Choose a session mode:
   - **Shared session** reuses one durable session for this trigger. Use it when later runs should see the trigger's previous conversation.
   - **New session per run** creates a fresh session every time. Use it when each run should be isolated.
6. Enter the message that starts the run. Messages may use `{{...}}` template values from the agent, trigger, and invocation context.
7. Leave **Enabled** on to activate the schedule, then select **Create trigger**.

The trigger card shows the schedule in human-readable form, its timezone, message, session mode, enabled state, and recent run outcomes.

## Operate and test a trigger

From the **Triggers** section of the **Integrations** tab you can:

- enable or disable the recurring schedule;
- edit its schedule, timezone, session mode, message, or enabled state;
- select **Run now** to start one invocation immediately;
- inspect the most recent durable execution outcomes; or
- delete the trigger.

**Run now** uses the same execution path as a scheduled run. The trigger must be enabled.

## API

Agent triggers are managed below `/v1/agents/{agent_id}/triggers`:

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/v1/agents/{agent_id}/triggers` | List triggers |
| `POST` | `/v1/agents/{agent_id}/triggers` | Create a schedule trigger |
| `GET` | `/v1/agents/{agent_id}/triggers/{trigger_id}` | Get one trigger |
| `PATCH` | `/v1/agents/{agent_id}/triggers/{trigger_id}` | Update provided fields |
| `DELETE` | `/v1/agents/{agent_id}/triggers/{trigger_id}` | Delete a trigger |
| `POST` | `/v1/agents/{agent_id}/triggers/{trigger_id}/trigger` | Run it now |
| `GET` | `/v1/agents/{agent_id}/triggers/{trigger_id}/runs` | List recent outcomes |

Schedule create requests accept `trigger_type: "schedule"`, `cron_expression`, `timezone`, `session_mode`, `message`, and `enabled`. `GET /v1/agents/{agent_id}/triggers/{trigger_id}/deliveries` lists recent deliveries, including ones that were filtered out or deduplicated. See the [API reference](/api/) for current request and response schemas.

## Run a saved script

A schedule or webhook trigger can run one of the Agent's
[saved scripts](/capabilities/tools-in-shell/#saved-scripts) instead of asking
the model. Set `script` on the trigger:

```json
{
  "trigger_type": "schedule",
  "cron_expression": "0 9 * * 1-5",
  "message": "Daily triage",
  "script": { "script": "triage-prs", "input": { "repo": "acme/web" } }
}
```

- When the trigger fires, its message is recorded in the session as usual, and
  the turn runs `tools scripts triage-prs` with the input on stdin. No model
  call is made.
- The run appears in the session as one `bash` tool call, with its result.
- The Agent needs [Tools in Shell](/capabilities/tools-in-shell/). The script
  must exist when the trigger is saved.
- No one is there to answer an approval. A call that needs approval stops the
  script, and the stop is recorded in the result.
- With `"wake_agent_on_failure": true`, a run that fails or stops hands its
  result to the Agent's model, which answers in the same turn. Without it, the
  run only records the result.
- Input strings are templates over the event, like the message. A string that
  is only one placeholder passes that value as is, so on a webhook trigger
  `{"pr": "{{payload.pull_request}}"}` gives the script the request's
  `pull_request` object; any other string, such as `"#{{payload.number}}"`, is
  rendered as text. A missing value becomes `null`.
- To make the trigger ask the model again, update it with `"script": {"script": ""}`.

## Run in a Slack conversation

A schedule trigger can run in a conversation of one of the Agent's own
[Slack](/capabilities/slack/) channels instead of a session of its own, such as
a 07:00 briefing in the direct message where a person already talks to the
Agent. Set `target_channel_id` and `target_binding_key` on create or update:

```json
{
  "trigger_type": "schedule",
  "cron_expression": "0 7 * * *",
  "timezone": "America/Chicago",
  "message": "Morning briefing",
  "target_channel_id": "appchan_01933b5a000070008000000000000001",
  "target_binding_key": "slack:user:U0123ABCD"
}
```

- The binding key names the conversation the way the channel keys its
  sessions: `slack:user:{user_id}` on a `per_user` channel (a direct message
  with that person) or `slack:channel:{channel_id}` on a `per_channel` channel
  (a top-level post).
- Each run lands in the session a Slack message from that conversation would,
  so the run and the conversation share one history. The first run creates and
  binds it, even before the person has written, and opens the direct message.
- The run posts its replies there like an answer to a Slack message, and the
  Agent's `send_message` reaches the same conversation. Posts are top-level,
  never in a thread.
- Only the Agent's own Slack channel can be targeted, by someone who can manage
  the Agent. If the channel is disabled, retired or no longer keys sessions
  that way, the run fails and its delivery records why. It never falls back to
  a session of its own.
- `session_mode` does not apply, and the trigger cannot also run a saved
  script. Update with `"target_channel_id": ""` to remove the target.

## Migrated App Schedules

The retired App model allowed `schedule` channels. Everruns migrated Agent-bound schedules to Agent triggers and preserved their cron expression, timezone, session mode, message, execution identity, and history.

Agent triggers, webhook triggers, and session schedules solve different problems:

- use an **agent trigger** when an agent should wake itself on a recurring schedule;
- use a **webhook trigger** when an external HTTP request should invoke an Agent; and
- use a **session schedule** when the current conversation should continue later.

## See also

- [Slack Integration](/capabilities/slack/), publish an Agent to an inbound messaging endpoint.
- [Session participants](/features/session-participants/), understand the host agent used by a trigger-created session.
- [API reference](/api/), exact trigger schemas and responses.
