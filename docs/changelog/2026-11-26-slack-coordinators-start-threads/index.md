---
title: Coordinators in Slack can start threads, and GitHub writes go through
description: A coordinator answering in Slack, or running a trigger aimed at Slack, now starts threads, and its GitHub writes no longer stall on an unseen form.
date: 2026-11-26
kind: fix
significance: notable
products: [platform, cloud]
surfaces: [agent]
links:
  docs: [/capabilities/threads/, /capabilities/slack/]
---

Ask a coordinator agent in [Slack](/capabilities/slack/) to hand a pull request
to a worker, and it used to refuse: a Slack member is not an organization user,
so every `start_thread` went looking for one and failed. A schedule aimed at
the same conversation hit the same wall. Now the agent starts threads there,
limited to the agents its [Threads](/capabilities/threads/) settings name, and
threads stay out of the channel.

GitHub writes made as the agent's own GitHub App, such as closing an issue,
used to stop at a confirmation form only a person in a browser could submit.
In Slack or a scheduled run nobody saw it. They now run directly, still behind
your Everruns tool approvals.
