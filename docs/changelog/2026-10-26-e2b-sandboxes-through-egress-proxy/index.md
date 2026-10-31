---
title: E2B sandbox traffic now follows your org's network rules
description: E2B sandboxes can send their traffic through the Everruns egress proxy, so it gets the same address, blocks, caps and usage counts as agent tools.
date: 2026-10-26
kind: feature
significance: minor
products: [platform]
release: "0.50.0"
surfaces: [operations]
links:
  docs: [/sre/environment-variables/, /features/sandbox-templates/#outbound-network]
  prs: [everruns/everruns#4931, everruns/everruns#4949]
---

Until now, an E2B sandbox reached the internet straight from E2B's network, outside the rules
Everruns applies to agent tools. It can now go through the Everruns egress proxy instead. Its
traffic leaves from the same address as your agents' tools, internal addresses and email (SMTP)
stay blocked, and the bytes count against your org's caps and usage.

To turn it on, enable the proxy's SOCKS5 listener, put TLS in front of it, and give the control
plane and workers its public address
([environment variables](/sre/environment-variables/#tenant-egress-proxy)):

```bash
# on the egress proxy
EGRESS_PROXY_SOCKS_ADDR=0.0.0.0:1080

# on the control plane and workers, next to EVERRUNS_EGRESS_PROXY_SECRET
EVERRUNS_EGRESS_PROXY_SOCKS_ADDRESS=egress.example.com:443
```

E2B still applies the [Sandbox Template's outbound rule](/features/sandbox-templates/#outbound-network)
and the Session's network access list first. Each sandbox gets its own credential, which E2B
holds and which never enters the sandbox. If the proxy is down or the credential has expired,
the sandbox loses network access instead of connecting directly. UDP, including DNS, still
goes out directly from E2B.
