---
title: E2B sandboxes can leave through the tenant egress proxy
description: With one setting, E2B sandbox traffic goes through everruns-egress-proxy, under the same per-org checks, caps and usage counts as agent tools.
date: 2026-10-26
kind: feature
significance: minor
products: [platform]
surfaces: [operations]
links:
  docs: [/sre/environment-variables/, /features/sandbox-templates/#outbound-network]
  prs: [everruns/everruns#4931]
---

`everruns-egress-proxy` has a SOCKS5 listener for sandbox providers. Every E2B sandbox that may
reach the network can now send its outbound TCP through the proxy. Sandbox traffic leaves from
the proxy's address, internal addresses and SMTP stay blocked, and the bytes count against the
org's caps, the same as agent tools.

Turn the listener on, put TLS in front of it, and give the control plane and workers its public
address ([environment variables](/sre/environment-variables/#tenant-egress-proxy)):

```bash
# everruns-egress-proxy
EGRESS_PROXY_SOCKS_ADDR=0.0.0.0:1080

# control plane and workers, next to EVERRUNS_EGRESS_PROXY_SECRET
EVERRUNS_EGRESS_PROXY_SOCKS_ADDRESS=egress.example.com:1080
```

E2B still applies the [Sandbox Template's outbound rule](/features/sandbox-templates/#outbound-network)
and the Session's network access list first. Each sandbox gets its own credential, which E2B
holds and which never enters the sandbox. If the proxy is down or the credential has expired,
the sandbox loses network access rather than connecting directly. UDP, including DNS, still
leaves directly from E2B.
