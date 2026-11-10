---
title: A CLI you can read at a glance
description: docker up and k8s up now show checks, answers and live pod progress in color and end with one box holding the URL, sign-in and next command; NO_COLOR is honored.
date: 2026-11-10
kind: improvement
significance: minor
products: [platform]
surfaces: [cli]
links:
  docs: [/features/cli/, /reference/cli/, /getting-started/kubernetes/]
---

Setting up Everruns with [`everruns docker up`](/getting-started/docker-compose/)
or [`everruns k8s up`](/getting-started/kubernetes/) now reads like a checklist.
Checks and answers get a green ✔, problems a yellow ! or red ✗ with the fix
under them, and the run ends in one box with the address, how to sign in and
the next command to type.

![everruns docker up: checks, answered questions, containers starting, and a summary box with the UI address and password](./docker-up@2x.png)

`k8s up` shows each workload's ready count while Helm waits (`worker 0/1 ·
server 1/2`), so a slow rollout says what it is waiting on. The new
`everruns k8s password` prints the generated admin password instead of a long
`kubectl` line. Both commands also offer OpenRouter as the LLM provider.

The rest of the [CLI](/features/cli/) follows suit: status pages, sign-in,
`chat`, tables and errors. Color is off when output is piped, with `--no-color`
or `--color never`, and with the usual `NO_COLOR`, `CLICOLOR=0` and
`TERM=dumb`; `CLICOLOR_FORCE=1` keeps it on.
