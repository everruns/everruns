---
title: Threat-intelligence deny feeds and abuse signals for outbound traffic
description: Self-hosted deployments can refuse domains on public threat-intelligence lists for all agent traffic, and the tenant egress proxy reports per-org abuse signals.
date: 2026-10-26
kind: improvement
significance: minor
products: [platform]
surfaces: [operations]
links:
  docs: [/sre/environment-variables/]
---

Set `EVERRUNS_EGRESS_DENY_FEEDS` on the control plane and workers to refuse
agent requests to domains on public threat-intelligence lists: malware,
phishing, scam and command-and-control hosts. The preset `hagezi-tif-mini`
loads the HaGeZi Threat Intelligence Feeds list, about 200,000 domains; your
own `https://` lists work too. A listed domain and its subdomains are refused
for every org, open egress included, unless your curated allowlist names the
host. Lists are fetched at start and every 6 hours, and a failed fetch keeps
the previous list.

```bash
EVERRUNS_EGRESS_DENY_FEEDS=hagezi-tif-mini
```

The tenant egress proxy now also counts, per org and minute, attempts on
blocked SMTP ports, destinations that resolve to private addresses, requests
refused by a cap, and how many distinct hosts the org reached. It sends them to
the control endpoint with usage, or logs them, so you can act on an org whose
traffic looks like spam or scanning.

See [Outbound egress variables](/sre/environment-variables/#outbound-egress).
