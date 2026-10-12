---
title: "GitHub triggers narrow CI events by branch and workflow"
description: "The GitHub trigger form takes the branches and workflow names a CI trigger accepts, offers one session for every event, and the agent builder fills both in."
date: 2026-10-12
kind: improvement
significance: minor
products: [platform, cloud]
surfaces: [ui]
links:
  docs: [/features/agent-triggers/]
  prs: [everruns/everruns#4607]
---

When a GitHub trigger listens to CI events, its form now has **Branches** and
**Workflows** fields. List `main` and `CI` to wake the agent only for the CI
workflow on main; leave a field empty to accept every branch or workflow.

The session mode list adds **One session for everything**, so a watcher that
reacts to every CI result keeps one ongoing conversation.

When you describe a CI watcher to the agent builder and name a branch or
workflow, the trigger it sets up arrives with those fields filled in.

See [Agent triggers](/features/agent-triggers/) and [#4607](https://github.com/everruns/everruns/pull/4607).
