---
title: Run agent sandboxes as their own VMs on your Kubernetes cluster
description: Isolated sandboxes put each agent sandbox in its own pod, and with AWS Nitro Sandboxes, gVisor or Kata its own kernel, with no provider account.
date: 2026-11-15
kind: feature
significance: notable
products: [platform]
surfaces: [ui, api, operations]
links:
  docs: [/features/isolated-sandboxes/, /getting-started/kubernetes/]
---

A self-hosted Everruns can now run real sandboxes itself. Turn on
`sandboxes.kubernetes` in the Helm chart and **Isolated sandbox** appears under
**Runs in**, with no provider account to connect. Each agent sandbox becomes one
pod with a workspace volume and its own network policy. Set the runtime class to
`isolated-sandbox` on EKS and every sandbox runs in its own AWS Nitro VM; `gvisor`
and `kata` do the same on other clusters. Without one, the template editor tells
you plainly that a container is no boundary for untrusted code.

```yaml
sandboxes:
  kubernetes:
    enabled: true
    runtimeClassName: isolated-sandbox
egressProxy:
  enabled: true
```

Egress is enforced by the cluster, outside the sandbox: through the egress
proxy, to listed IP ranges, or nowhere. An idle sandbox gives back its compute
and keeps its files. Sandboxes start from `ghcr.io/everruns/sandbox-dev`
(Node, Python, Go, Rust, Docker and GitHub CLIs) or `sandbox-desktop`, which
adds a screen the agent can see and drive. Any other image works once the
`sandbox-guest` Dev Container Feature is added. See
[Isolated sandboxes](/features/isolated-sandboxes/).
