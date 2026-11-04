---
type: Specification
title: "Coding Agent Onboarding"
description: "Connect an external coding agent through one setup prompt while preserving provider setup and honest connection status."
tags:
  - everruns
  - ui
  - onboarding
  - mcp
---

# Coding Agent Onboarding

Connecting an external coding agent is a personal way to use Everruns. It is
available from the user menu, the Agents first-use state and the existing onboarding
Done screen through one [shared dialog](../../apps/ui/src/components/layout/mcp-connect-button.tsx).
There is no host selector, extra signup step or contextual task handoff.

The Agents first-use entry remains visible when only built-in agents exist:
signup ensures Platform Chat, which otherwise makes an empty-state-only entry
unreachable. The built-in agent stays in the operational list.

The setup prompt explicitly carries this deployment's MCP endpoint and authentication
mode. The distributed plugin defaults to Cloud, so a self-hosted installation must
not silently send its user to Cloud. The public bootstrap at
<https://everruns.com/agent-setup/prompt.md> is owned by the landing repository; it
uses the plugins repository's maintained host instructions and finishes with a
read-only identity check. The UI embeds no secrets in the prompt.

## Providers remain a separate prerequisite

The existing Configure step handles providers before Done. Connecting a coding
agent does not provide model credentials for Everruns runtime agents or Platform
Chat. With a provider, Done keeps Platform Chat as its primary action. After a
skip, its primary action returns to Models → Providers and its copy states what
is missing. Both variants offer coding-agent connection for setup and management.
The owning page is `apps/ui/src/app/(main)/orgs/[orgId]/setup/page.tsx` and uses
the [shared onboarding shell](../../apps/ui/src/components/onboarding/onboarding-shell.tsx).

## Honest connection status

The dialog stays focused on one setup prompt and its copy action. Claude Code,
Codex, Cursor and Gemini CLI marks identify compatible agents without becoming a
selector. Manual installation remains a small documentation link. OAuth guidance
and local no-auth instructions live in the deployment-specific prompt.

Connection management stays in Settings → AI clients, where users can refresh
OAuth approvals and revoke them. The dialog has
no separate authorization step, configuration accordion or provider explanation;
provider setup remains in the existing onboarding flow.

There is no waiting spinner or success claim based on opening the dialog or
copying the prompt. An OAuth grant records approval; last use records an MCP
request, not plugin installation. The UI does not correlate a setup attempt
with a grant. See [Connected AI Clients](../integrations/mcp-connected-clients.md).
