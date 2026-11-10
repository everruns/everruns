---
title: A CLI you can read at a glance
description: docker up and k8s up show checks, answers and live pod progress in color, then end with one box holding the URL, sign-in and next command.
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
Checks and answers get a green ✔, warnings stand out in yellow and failures in red with the fix
under them, and the run ends in one box with the address, how to sign in and
the next command to type. With a single admin sign-in, the box shows the
password itself.

![everruns docker up: checks, answered questions, containers starting, and a summary box with the UI address and password](./docker-up@2x.png)

`k8s up` shows each workload's ready count while Helm waits (`worker 0/1 ·
server 1/2`), so a slow rollout says what it is waiting on. The new
`everruns k8s password` prints the generated admin password instead of a long
`kubectl` line. Both commands also offer OpenRouter as the LLM provider.

`reset` is now `destroy`, in both `docker` and `k8s`, since it deletes the
install rather than clearing its data. On Kubernetes it also removes the
namespace once nothing but Kubernetes' own defaults is left in it.

The rest of the [CLI](/features/cli/) follows suit ([every command](/reference/cli/)): status pages, sign-in,
`chat`, tables and errors. Color is off when output is piped, with `--no-color`
or `--color never`, and with the usual `NO_COLOR`, `CLICOLOR=0` and
`TERM=dumb`; `CLICOLOR_FORCE=1` keeps it on.
