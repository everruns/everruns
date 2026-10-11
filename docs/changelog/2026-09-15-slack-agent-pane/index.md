---
title: Everruns agents run in Slack's agent pane
description: A Slack channel can now serve Slack's agent pane, with streamed replies, a stop button, live status and thread context, alongside channel replies.
date: 2026-09-15
kind: feature
significance: major
products: [platform, cloud]
surfaces: [agent, operations]
release: "0.27.0"
links:
  docs: [/capabilities/slack/, /how-to/publish-to-slack/]
  prs: [everruns/everruns#3560, everruns/everruns#3564, everruns/everruns#3568, everruns/everruns#3581, everruns/everruns#3567, everruns/everruns#3563, everruns/everruns#3606]
---

Set `agent_surface_enabled` on a Slack channel and its app also appears as an
agent in Slack's dedicated assistant pane, while `@mentions` in channels keep
working. In the pane, replies stream as the agent writes them, the stop button
cancels the running turn, and the agent's status and thread title show while a
turn runs. Thread context is persisted and given to the agent, and replies post
as markdown blocks.

The generated manifest declares the agent feature and fills Slack's suggested
prompts from the agent's conversation starters, or the harness's. An existing
app must update its manifest and reinstall to gain the pane.

See [Slack](/capabilities/slack/#agent-surface) and
[Publish to Slack](/how-to/publish-to-slack/). [#3560](https://github.com/everruns/everruns/pull/3560), [#3564](https://github.com/everruns/everruns/pull/3564), [#3606](https://github.com/everruns/everruns/pull/3606)
