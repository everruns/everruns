---
title: "Session traces report shell exits and fetch failures"
description: "Trace API responses expose command exit codes, and the runtime preserves settled shell output and reports why outbound requests fail."
date: 2026-11-21
kind: fix
significance: minor
products: [platform, cloud]
surfaces: [api, agent]
links:
  docs: [/features/ui/#sessions]
---

A shell tool can successfully capture the output of a command that exited
with an error. Read `command_outcome.exit_code` on the Trace API's step
detail to distinguish a nonzero exit from a failed invocation. On workers
with durable tool results, a restarted turn can recover the recorded
output without rerunning the command.

Web fetch errors distinguish connection failures, timeouts and network
policy denials. The built-in model provider accepts calls after
compaction falls back to local history processing. See
[Sessions](/features/ui/#sessions) for the Trace workflow.
