---
title: Agent Triggers
description: Run an agent proactively on a recurring schedule, choose session reuse, test it immediately, and inspect recent outcomes.
---

Agent triggers let an agent start work on its own schedule. A trigger belongs to one agent, runs on that agent's harness, and sends a configured message when it fires. You do not need to put an App in front of the agent.

Schedule triggers are the only trigger type currently available.

## Create a trigger in the UI

1. Open the agent.
2. Select the **Triggers** tab.
3. Select **Add trigger**.
4. Choose a preset or enter a 5-field cron expression (or a 7-field expression whose year is `*`), then enter an IANA timezone. The editor shows the schedule in human-readable form and previews the next eight runs. The default timezone is `UTC`.
5. Choose a session mode:
   - **Shared session** reuses one durable session for this trigger. Use it when later runs should see the trigger's previous conversation.
   - **New session per run** creates a fresh session every time. Use it when each run should be isolated.
6. Enter the message that starts the run. Messages may use `{{...}}` template values from the agent, trigger, and invocation context.
7. Leave **Enabled** on to activate the schedule, then select **Create trigger**.

The trigger card shows the schedule in human-readable form, its timezone, message, session mode, enabled state, and recent run outcomes.

## Operate and test a trigger

From the **Triggers** tab you can:

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

## Migrate from App schedule channels

The App `schedule` channel is deprecated, and new schedule channels are rejected. Create a schedule trigger on the App's agent instead.

Existing agent-bound App schedule channels are migrated automatically. Their cron expression, timezone, session mode, and message are preserved, and active schedules retain their durable execution identity and history. Existing Apps without an agent are grandfathered and are not migrated automatically.

App webhook channels and session schedules solve different problems and are unchanged:

- use an **agent trigger** when an agent should wake itself on a recurring schedule;
- use an **App webhook channel** when an external HTTP request should invoke an App; and
- use a **session schedule** when the current conversation should continue later.

## See also

- [Apps](/features/apps/), publish agents to inbound channels.
- [Session participants](/features/session-participants/), understand the host agent used by a trigger-created session.
- [API reference](/api/), exact trigger schemas and responses.
