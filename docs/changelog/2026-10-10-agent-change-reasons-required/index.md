---
title: "Agents must give a reason for every change by default"
description: "Require reasons from agents is now on by default, so an agent's change without a reason fails with reason_required; organizations can turn the flag off."
date: 2026-10-10
kind: breaking
significance: minor
products: [platform, cloud]
surfaces: [agent, operations]
links:
  docs: [/features/change-history/#reasons]
  prs: [everruns/everruns#4271, everruns/everruns#4552]
---

A change an agent makes without a reason is now refused by default: the command
fails with the `reason_required` code and a `retry` action, and the agent
retries with `--reason`. Built-in tools that change sessions on the agent's
behalf, such as `spawn_agent` and handoffs, give their own reasons, so they keep
working.

This is the **Require reasons from agents** feature flag, now on by default. An
organization can turn it off to let such changes through with a warning, which
the `everruns_entity_changes_without_reason_total` metric counts. People in the
UI and scripts calling the API may still leave the reason out.

See [Reasons](/features/change-history/#reasons) and [#4271](https://github.com/everruns/everruns/pull/4271), [#4552](https://github.com/everruns/everruns/pull/4552).
