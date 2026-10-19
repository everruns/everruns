---
title: Tenant egress proxy for self-hosted deployments
description: Self-hosted deployments can send agent outbound traffic through everruns-egress-proxy for its own outbound address, per-org caps and per-org usage counts.
date: 2026-10-18
kind: feature
significance: notable
products: [platform]
surfaces: [operations]
links:
  docs: [/advanced/physical-architecture/, /sre/environment-variables/]
---

The server image now ships `everruns-egress-proxy`. Point the control plane and
workers at it with `EVERRUNS_EGRESS_PROXY_URL` and a shared
`EVERRUNS_EGRESS_PROXY_SECRET`, and traffic that agents, capabilities, MCP
servers and integrations send leaves through it. Calls the deployment makes
itself, such as LLM providers and email, keep leaving directly.

Run the proxy on a host with its own public address, and an address blocked
because of one tenant's traffic does not take model calls down with it. The
proxy checks a short-lived token naming the org and session of every request,
refuses private addresses and SMTP ports, and applies the per-org caps you set
(requests per minute, per host, open connections, bytes per day; all off by
default). It logs bytes and requests per org and session every minute, or sends
them to a control endpoint that can also supply caps per org.

```bash
# Proxy host
EVERRUNS_EGRESS_PROXY_SECRET=<32+ random bytes>
EGRESS_PROXY_ORG_RPM=600
everruns-egress-proxy

# Control plane and workers
EVERRUNS_EGRESS_PROXY_URL=http://egress-proxy:4750
EVERRUNS_EGRESS_PROXY_SECRET=<same value>
```

See [Outbound traffic](/advanced/physical-architecture/#outbound-traffic) and
[Tenant egress proxy variables](/sre/environment-variables/#tenant-egress-proxy).
