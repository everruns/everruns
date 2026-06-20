---
title: Cursor Cloud Agents for Everruns Workflows
description: Launch and manage Cursor Cloud Agents from Everruns agents.
sidebar:
  label: Cursor
---

<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="56" height="56" aria-hidden="true" style="float: right; margin-left: 16px;"><path d="M5 3l14 7-6 2.2L10.5 19z" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>

Everruns integrates with [Cursor Cloud Agents](https://docs.cursor.com/en/background-agents) so agents can delegate asynchronous coding work to Cursor. A triage agent can inspect a request, split it into focused tasks, launch Cursor agents against GitHub repositories, send follow-ups, and summarize status or results.

## What You Get

- **Launch Cloud Agents**: Start Cursor agents with repository, base ref, task prompt, optional branch name, and PR behavior.
- **Track Progress**: Read status, target branch, PR URL, summary, and conversation history.
- **Send Follow-ups**: Add more instructions to running Cursor agents.
- **Connection Prompt**: Configure a Cursor Cloud Agents API key in Settings > Connections.
- **Seed Agent**: Use the built-in **Cursor Agent Manager** example to triage and delegate work.

## Quick Start

### 1. Get a Cursor API Key

1. Open [Cursor Dashboard](https://cursor.com/dashboard?tab=cloud-agents)
2. Go to **Cloud Agents** > **My Settings** > **API Keys**
3. Create a Cloud Agents API key
4. Make sure Cursor's GitHub app can access the repositories agents should work on

Use a Cloud Agents API key. A general Cursor dashboard API key may not be enough to create agents.

### 2. Connect in Everruns

1. Go to **Settings** > **Connections**
2. Find **Cursor**
3. Click **Connect**
4. Paste the Cloud Agents API key

### 3. Use in Sessions

Agents with the Cursor capability can use these tools:

| Tool | Description |
|------|-------------|
| `cursor_launch_agent` | Start a Cursor Cloud Agent |
| `cursor_get_agent` | Get one agent's status and result metadata |
| `cursor_list_agents` | List agents for the connected Cursor account |
| `cursor_add_followup` | Send extra instructions to a running agent |
| `cursor_get_conversation` | Read an agent's conversation transcript |
| `cursor_delete_agent` | Delete an agent record/resources |
| `cursor_list_models` | List recommended Cursor model ids |
| `cursor_list_repositories` | List GitHub repositories Cursor can access |
| `cursor_key_info` | Check the active Cursor connection |

## Delegation Pattern

Use **Cursor Agent Manager** when you want Everruns to triage first and then delegate:

1. Provide the repository URL and base branch
2. Ask the agent to break the work into scoped tasks
3. Let it launch Cursor agents with clear acceptance criteria
4. Review returned Cursor links, branches, PR URLs, summaries, and transcripts

## Lifecycle

Cursor owns Cloud Agent lifecycle state. Everruns stores no per-agent state beyond normal session messages and tool results. Keep the returned `agent_id` if you want to poll, send follow-ups, read the conversation, or delete the agent later.

`cursor_list_repositories` is heavily rate-limited by Cursor and can be slow for large accounts. Prefer passing the repository URL directly.

## Security

- Cursor API keys are encrypted at rest when stored as Everruns user connections
- Everruns prompts for missing Cursor credentials through the inline connection dialog, not chat text
- Cursor agents run in Cursor-managed remote environments with internet access and command execution
- Repository access is controlled by the Cursor GitHub app and Cursor account settings
- Webhook secrets and image prompt payloads are intentionally not exposed in the first tool surface

## Links

- [Cursor Cloud Agents](https://docs.cursor.com/en/background-agents)
- [Cursor Background Agents API](https://docs.cursor.com/en/background-agent/api/overview)
- [Launch Agent API](https://docs.cursor.com/en/background-agent/api/launch-an-agent)
- [Cursor GitHub app](https://docs.cursor.com/en/github)
