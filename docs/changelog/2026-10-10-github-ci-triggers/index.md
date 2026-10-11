---
title: "GitHub triggers wake agents on CI results"
description: "GitHub triggers now receive workflow run and check suite completions, and a changes-only filter wakes the agent only when a workflow's CI result changes."
date: 2026-10-10
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/features/agent-triggers/, /how-to/summarize-github-pull-requests/]
  prs: [everruns/everruns#4541, everruns/everruns#4546]
visual: "GitHub trigger form with the Only when the CI result changes option (/agents/<id>), cropped to the form"
---

A GitHub trigger now fires on `workflow_run` and `check_suite` completions, with
a normalized `github.ci` context for the message and each workflow as its own
subject. New agent GitHub Apps request read access to actions and checks; the
agent's GitHub card lists what an older App is missing.

Turn on **Only when the CI result changes** in the GitHub trigger form to wake
the agent only when the outcome for a workflow and branch changes, for example
from passing to failing, instead of on every run. Triggers expose this as the
`changes_only` filter.

See [Agent triggers](/features/agent-triggers/),
[Summarize GitHub pull requests](/how-to/summarize-github-pull-requests/) and [#4541](https://github.com/everruns/everruns/pull/4541), [#4546](https://github.com/everruns/everruns/pull/4546).
