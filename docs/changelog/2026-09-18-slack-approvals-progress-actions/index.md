---
title: Slack agents ask approval with buttons and show task progress
description: Agents deployed to Slack can take approvals through Approve and Decline buttons, show live sub-task progress and act as the channel's own bot.
date: 2026-09-18
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [agent]
release: "0.30.0"
links:
  docs: [/capabilities/slack/]
  prs: [everruns/everruns#3696, everruns/everruns#3697, everruns/everruns#3693]
---

Three additions for agents running in Slack:

- **Approvals:** when an agent pauses with `request_approval`, the thread shows
  buttons. Only the person the agent is answering can use them, and answering
  removes the buttons. Apps created earlier must save a fresh manifest once.
- **Task progress:** a delegating agent posts one message, updated in place, that
  summarizes its sub-tasks, for example how many finished or failed.
- **Slack actions:** a native `slack` capability adds `slack_add_reaction`,
  `slack_update_message`, `slack_lookup_user` and `slack_upload_file`, using the
  endpoint's own bot token instead of a separate Slack MCP server.

See [Slack approvals](/capabilities/slack/#approvals), [task progress](/capabilities/slack/#task-progress)
and [Slack actions](/capabilities/slack/#slack-actions). [#3696](https://github.com/everruns/everruns/pull/3696), [#3697](https://github.com/everruns/everruns/pull/3697), [#3693](https://github.com/everruns/everruns/pull/3693)
