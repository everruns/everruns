---
title: Scheduled turns in a Slack conversation post to it
description: A session schedule that fires in a Slack-bound session can now send_message to that Slack conversation instead of failing with no Slack session.
date: 2026-10-12
kind: fix
significance: notable
products: [platform, cloud]
surfaces: [agent]
release: "0.48.0"
links:
  docs: [/capabilities/slack/]
---

An Agent that set itself a schedule with `create_schedule` inside a Slack
conversation, such as a 07:00 briefing in a direct message, could not reach
Slack when the schedule fired: `send_message` failed because the scheduled
input carries no Slack message, so the post stayed in the session.

A scheduled turn now posts to the conversation of the session's latest Slack
message. In a direct message it posts inline, in a `per_channel` session it
posts a new top-level message rather than into the last thread, and a
`per_thread` session keeps its thread. The target comes only from signed Slack
input, never from the schedule's text, and the turn still shows as scheduled.
A session that has never received a Slack message behaves as before. See
[How the Agent Replies](/capabilities/slack/#how-the-agent-replies).

Watch the [Kids Duties walkthrough](https://www.youtube.com/watch?v=mbbYFif9juY):
a saved run and actual 07:00 Slack briefing, with English narration and captions.
