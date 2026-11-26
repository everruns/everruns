---
title: Coordinators in Slack can start threads, and GitHub writes go through
description: A coordinator answering in Slack, or running a trigger aimed at a Slack conversation, now starts threads, and its GitHub writes no longer stall on an unseen form.
date: 2026-11-26
kind: fix
significance: notable
products: [platform, cloud]
surfaces: [agent]
links:
  docs: [/capabilities/threads/, /capabilities/slack/]
---

Ask a coordinator agent in Slack to hand a pull request to a worker, and it
used to say it could not: every `start_thread` from a Slack message was
refused, because a Slack member is not an organization user and the command
went looking for one. A schedule aimed at the same Slack conversation hit the
same wall, so a coordinator that lived in a channel could triage but never
delegate.

Now the agent starts threads there just as it does in the console, limited to
the agents its [Threads](/capabilities/threads/) settings name. Threads stay
out of the channel, so only the coordinator answers in Slack.

Closing an issue or editing a pull request through GitHub's MCP server as the
agent's own GitHub App also used to stop halfway: GitHub showed a confirmation
form that only a person in a browser could submit, and in Slack or a scheduled
run nobody ever saw it. Those writes now run directly, still behind your
Everruns tool approvals.
