---
title: Route Kubernetes Ingress directly to Everruns
description: The Everruns Helm chart routes directly to application services when Ingress is enabled, and k8s up can sign in while the public endpoint is coming up.
date: 2026-11-20
kind: feature
significance: minor
products: [platform]
surfaces: [operations]
links:
  docs: [/getting-started/kubernetes/#ingress-routing]
---

Set `ingress.enabled=true` to route directly to Everruns through your cluster's
Ingress controller. The UI, API, MCP and discovery documents share a hostname.
Use `proxy.enabled=true` to keep Caddy behind an Ingress.

The CLI uses the same default:

```bash
everruns k8s up --public-url https://everruns.example.com
```

If the public endpoint is still coming up, the CLI signs in through a temporary
port-forward and saves the public URL in its profile. See
[Ingress routing](/getting-started/kubernetes/#ingress-routing) for controller settings.
