---
title: Local stress tests use successful offline tool workloads
description: Fix local proxy connection failures during stress tests, seed successful offline tool workloads, and improve latency and failure reporting.
date: 2026-10-14
kind: fix
significance: minor
products: [platform, framework]
release: "0.49.0"
surfaces: [operations, code]
---

Local proxy connections use IPv4 loopback, fixing HTTP 502 responses during
concurrent request bursts. Failed API connection attempts have bounded retries; POSTs already
received by the API are not retried. Local setup installs Caddy 2.11.4, and
startup rejects older versions that lose POST bodies on failed dials.

The simulated load driver uses a seeded session file for its default research
workload, exercising three rounds of successful offline tools. Deferred tools
also retain their original argument schemas in llmsim. The driver checks that
the selected model uses llmsim, waits for SSE connectivity, reports p95 and
average latency, and returns a failing exit status for incomplete runs.
Bounded HTTP connection pools keep large runs from stalling the load
generator and inflating client measurements.

Run `python3 scripts/load-test.py --model <enabled-llmsim-model-id>`. Keep
the scenario mix with results: the file workload differs from earlier runs
that chose arbitrary read tools.
