---
title: Management UI
description: Manage agents, sessions, capabilities, settings, files, and event streams through the optional web interface.
appliesTo: [platform, cloud]
---

While Everruns is a headless agent platform designed for API-first integration, it provides an optional management UI for administrative tasks and session monitoring.

## Overview

The management UI is a Next.js application that provides:

- Agent management (create, edit, delete)
- Session monitoring and chat interface
- Capabilities browser
- Settings management (LLM providers, personal access tokens, team members)
- Dashboard with system statistics

Access the UI at `http://localhost:9300` when running locally.

## Navigation

The sidebar provides access to main sections:

| Section | Description |
|---------|-------------|
| Dashboard | Overview statistics and quick actions |
| Agents | List, create, and manage agents |
| Virtual users | Manage end-user and service runtime accounts |
| Capabilities | Browse available capabilities |
| Settings | Configure providers, personal access tokens, and team members |

## Dashboard

The dashboard provides an at-a-glance view of your system:

- **Stats Cards**: Total agents, active sessions, and other metrics
- **Recent Agents**: Quick access to recently created or updated agents
- **Quick Actions**: Shortcuts to create agents or browse the agent list

## Agents

### Agent List

The agents page displays all agents in a card grid layout.

![Agents page showing card grid layout](../images/features/ui.png)

Each card shows:

- Agent name and status badge (active/inactive)
- Truncated ID
- Description preview
- Enabled capabilities with icons
- Tags
- Creation date
- Edit button

Click a card to view the agent details, or click the edit icon to modify the agent.

### Agent Page

The agent page reads and edits an agent in one layout. The system prompt fills the wide left pane;
a narrow column on the right holds the settings:

- **Harness**, **Capabilities** (in precedence order), **Default model**, and **Tags**
- **Updated**, read-only
- **More**: one row each for Branding, MCP servers, Credentials, Starter files, Network access,
  Token usage, and Health check. Each row shows its current value and opens a side sheet.

Tabs: **Agent**, **Preview**, **Integrations** (endpoints and triggers), **Stats**, and
**Sessions**.

Header actions:
- **Edit**: switch the page into edit mode. The prompt becomes an editor and the settings take
  input; **Save changes** sends everything at once and **Discard** drops the draft. Changes apply
  to new sessions only.
- **More actions**: Copy, Export, Version history, and Archive (or Delete, for an archived agent)
- **Test chat**: start an interactive chat thread with this agent

## Sessions

### Session View

Each session has three tabs:

#### Chat Tab

The primary interface for viewing and participating in conversations:

- Message history with user messages (dark bubbles) and agent responses
- Tool call visualization with expandable details
- Tool results displayed inline
- Message input with keyboard shortcuts (Enter to send, Shift+Enter for newline)
- Reasoning effort selector (for models that support extended thinking: Anthropic Claude, OpenAI GPT-5.x, o-series)

#### File System Tab

Browse and manage files associated with the session's sandboxed environment.

#### Events Tab

View raw session events for debugging:

- Sequence number
- Event type (input.message, output.message.completed, tool.completed, etc.)
- Timestamp
- JSON data payload

### Session Status

Sessions display their current status:

| Status | Badge | Description |
|--------|-------|-------------|
| started | Outline | Newly created, no messages yet |
| idle | Secondary | Ready for input |
| active | Primary | Currently processing |

## Capabilities

The capabilities page lists all available functionality modules:

- **Summary Panel**: Counts by status (available, coming soon, deprecated)
- **Category Tags**: Filter by capability type
- **Capability Cards**: Click to view details including tools and configuration

Each capability card shows:
- Icon and name
- Identifier (for API use)
- Status badge
- Description
- Category tag

## Settings

### LLM Providers

Configure language model providers:

- Add provider credentials (API keys)
- Enable/disable specific models
- Set default models for agents

### Personal access tokens

Manage personal access tokens for programmatic access. Tokens are tied to your
user account (not an organization) and inherit access to every organization
available to your account:

- Create new personal access tokens
- View existing tokens (values hidden)
- Revoke tokens

### Members

View and manage team members (when authentication is enabled).

## Virtual users and personal settings

Open **Virtual users** to create or manage organization-scoped accounts. Choose **End user** for a person using agents or **Service** for an agent account. The detail page has **Overview**, **Connections**, **Linked identities**, and **Sessions** tabs. Usage is fixed at creation. An agent's overview lets you select its service account.

**Settings → Account** edits your Everruns management profile. **My agent experience** edits your current organization's default virtual-user profile and runtime defaults. **Connections** uses the same account and connection store. Switching organizations selects that organization's runtime account. **Team members** remains management membership administration.

End-user connections are private to their owner. Organization management permissions do not grant access to another end user's credentials. Service account connections require management permissions. Saved credentials are encrypted and never returned by the API.

When upgrading an account that belonged to several organizations, old connections require an explicit destination. Select the destination from **Connections** and move each pending connection once. A destination that already has that provider is rejected rather than overwritten. Accounts with one organization migrate automatically.

Operators upgrading an existing installation should follow the [virtual-user cutover runbook](/sre/runbooks/virtual-user-cutover/).
