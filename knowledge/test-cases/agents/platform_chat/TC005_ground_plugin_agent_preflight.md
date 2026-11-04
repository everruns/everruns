---
type: Test Case
title: "TC005: Ground Plugin and Connection State Before Agent Creation"
description: "Verify that Platform Chat distinguishes operation discovery from resource inspection and grounds an Agent-creation confirmation in authoritative plugin, capability, Agent, connector, and current-user connection reads."
tags:
  - everruns
  - test-case
  - agents
  - platform-chat
---
# TC005: Ground Plugin and Connection State Before Agent Creation

## Description

Verify that Platform Chat distinguishes operation discovery from resource
inspection and grounds an Agent-creation confirmation in authoritative plugin,
capability, Agent, connector, and current-user connection reads.

## Preconditions

- A DB-backed stack with `AUTH_MODE=none` and a real tool-calling model.
- The `resend` plugin is installed and active.
- One active Agent references the installed plugin's exact `plugin:plugin_…`
  capability ref.
- Run both connection variants: no current-user Resend OAuth connection, then
  a connected Resend OAuth account.

## Steps

1. Open Platform Chat — `/chats` → **New chat** → built-in **Platform Chat** harness — and send: `Create another agent with resend plugin.`
2. Before confirming, inspect the tool trace.
3. Verify bounded `tool_search` reveals the needed native schemas and Platform
   Chat reads all five authoritative views through prefixed native functions:
   installed plugins, capabilities, Agents, connection providers, and
   current-user connections. Each function receives its canonical typed input
   under `params`. `everruns` shell composition is allowed when useful and uses
   progressive `--help` with supported flags rather than guessed flags.
   Search results identify operations; they do not establish resource state.
4. Verify the response identifies the installed Resend plugin by name/link,
   reports it active/available and already attached to one Agent, and separately
   reports the current user's OAuth connection state.
5. Verify Platform Chat asks for confirmation before creating the reusable
   org-wide Agent. In the disconnected variant, the confirmation explains that
   the user must connect Resend before the Agent can use its OAuth-backed tools.
6. Repeat with two installed resources whose names both match a shared search
   term. Verify Platform Chat presents the candidates and asks which one to use;
   it does not guess.
7. Disable the target plugin and repeat. Verify Platform Chat reports the
   disabled state and does not propose attaching it as available.
8. Use a unique absent name and repeat. Verify absence is concluded only after
   authoritative resource reads, never from a zero-match `tool_search` result.

## Expected

- Installed, active/available, attached, and connected are independent facts.
- Connected and disconnected current-user states are accurate and secret-free.
- Disabled, absent, and ambiguous-name states are not collapsed together.
- No operation-catalog miss is presented as evidence that a resource is absent.
- Preflight is short and deterministic, preserves org/user boundaries, and
  shows names and useful UI links instead of raw IDs in prose.
- Agent creation remains behind explicit confirmation.
