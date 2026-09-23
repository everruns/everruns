---
title: Agent Triggers
description: Run an agent proactively on a recurring schedule, choose session reuse, test it immediately, and inspect recent outcomes.
---

Agent triggers let an Agent start work on its own schedule. A trigger belongs to one Agent, runs on that Agent's Harness, and sends a configured message when it fires. It does not require an inbound endpoint.

Schedule triggers are the only trigger type currently available.

## Create a trigger in the UI

1. Open the agent.
2. Select the **Integrations** tab.
3. Select **Add trigger**.
4. Choose a preset or enter a 5-field cron expression (or a 7-field expression whose year is `*`), then enter an IANA timezone. The editor shows the schedule in human-readable form and previews the next eight runs. The default timezone is `UTC`.
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

Create requests accept `cron_expression`, `timezone`, `session_mode`, `message`, and `enabled`. See the [API reference](/api/) for current request and response schemas.

## Migrated App Schedules

The retired App model allowed `schedule` channels. Everruns migrated Agent-bound schedules to Agent triggers and preserved their cron expression, timezone, session mode, message, execution identity, and history.

Agent triggers, webhook triggers, and session schedules solve different problems:

- use an **agent trigger** when an agent should wake itself on a recurring schedule;
- use a **webhook trigger** when an external HTTP request should invoke an Agent; and
- use a **session schedule** when the current conversation should continue later.

## See also

- [Slack Integration](/integrations/slack/), publish an Agent to an inbound messaging channel.
- [Session participants](/features/session-participants/), understand the host agent used by a trigger-created session.
- [API reference](/api/), exact trigger schemas and responses.
