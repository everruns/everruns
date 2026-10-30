---
title: Run shared coding tasks in Slack
description: Create a Slack Code channel with one durable Everruns session, review surfaces, and requester approvals.
slug: how-to/slack-code
appliesTo: [platform]
---

Slack Code gives a coding task its own channel. Teammates can add instructions in
that channel or its threads; all messages continue the same Everruns session and
sandbox. The person who started the task approves consequential tool calls.

## Enable Slack Code

1. Configure a coding agent with its repository and sandbox capabilities.
2. Open its **Integrations** tab, edit its Slack channel, and enable **Slack Code channels**.
3. Save the channel, update the Slack app with the generated manifest, and reinstall
   it. The manifest enables `features.code_channels.enabled` and adds
   `code_channels:manage`, `channels:read`, and `groups:read`. Slack must make Code
   available to the installation; enabling the Everruns setting alone is insufficient.
4. Publish the channel and mention the agent in an allowed source channel.

Use a dedicated endpoint for coding: when this option is enabled, channel mentions
start coding tasks. Direct messages keep their ordinary conversational behavior.
If the endpoint restricts its source channel, that restriction still applies.

## Work together

Everruns creates a fresh session before creating the Slack channel. Retries reuse
that binding. It sends only the initiating request into the new session, rather
than copying private conversation history. Slack inherits the origin's privacy;
the agent cannot request a public override.

Messages and thread replies in the task channel steer the same session. Follow-ups
in the initiating thread also reach it. Each message retains its Slack author.
Joining the channel does not grant dashboard access or another person's credentials.

Hard approval cards name a specific pending tool call. Only the original requester
can allow or reject it. Expired, replayed, or changed requests cannot release a
call. Credential-storage consent still requires the authenticated Everruns UI.

## Review and finish

The agent uses `slack_code_task` to publish complete context snapshots, a summary,
and a unified diff read from a session file. The context bar includes an Everruns
session link. Large diffs must be shared through review links; Slack's diff limit
is 1,000,000 bytes. Stale revisions are rejected, and rate limits retry durably.

The agent can mark its final summary complete. Once execution finishes, Everruns
posts that summary, closes the Slack session, and archives the channel. Unarchiving
reconnects to the same Everruns session. Sandbox availability follows the sandbox
provider's normal retention policy.

Stopping a task cancels execution and suppresses subsequent delivery, including
after a server restart. A fresh human instruction can resume it. Removing the app,
revoking its token, or disabling its endpoint stops delivery; reinstalling requires
verification of the installation before work can resume.

HTML views, shared editable canvases, delegated approvers, and native GitHub review
synchronization are outside this integration's initial scope.
