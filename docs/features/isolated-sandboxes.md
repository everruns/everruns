---
title: Isolated Sandboxes
description: Run each agent sandbox as its own pod on your Kubernetes cluster, in its own VM with AWS Nitro Sandboxes, gVisor or Kata, with no provider account.
appliesTo: [platform]
---

An **Isolated sandbox** is a Sandbox Template target that your Everruns deployment runs itself.
Each sandbox is one pod in a namespace you choose, with a workspace volume, its own network
policy and the Everruns sandbox guest as its entrypoint. There is no provider account to
connect: the server uses the deployment's own cluster access.

How strong the isolation is depends on the cluster. With a VM runtime class, every sandbox gets
its own kernel:

| Runtime class | Where | What each sandbox gets |
|---|---|---|
| `isolated-sandbox` | EKS Auto Mode with AWS Nitro Sandboxes | Its own Nitro VM |
| `gvisor` | Any cluster with gVisor installed | A user-space kernel |
| `kata` | Any cluster with Kata Containers | A lightweight VM |
| none | Any cluster | A plain container on the node's kernel |

Without a runtime class, the template editor says so plainly: a container is not a security
boundary for code you do not trust.

## Turn it on

Isolated sandboxes are part of the [Helm chart](/getting-started/kubernetes/). Turn them on with
the egress proxy, which open-network sandboxes connect through:

```yaml
sandboxes:
  kubernetes:
    enabled: true
    namespace: everruns-sandboxes
    runtimeClassName: isolated-sandbox   # or gvisor, kata, or "" for plain containers
    sizes: [small, medium, large]
    maxPerOrg: 5
egressProxy:
  enabled: true
```

The chart creates the namespace (with the `baseline` Pod Security level, so a sandbox cannot be
privileged), a Role that lets the server and workers manage pods, volume claims, secrets and
network policies in that namespace only, and a default-deny network policy. Nothing gets a
cluster-wide permission, and sandbox pods get no service account token.

Outside the chart, set `KUBERNETES_SANDBOX_NAMESPACE` on the server and workers; the target is on
when the server can reach Kubernetes and that variable is set. The other settings are listed in
[Environment variables](/sre/environment-variables/).

## Templates

Pick **Isolated sandbox** under **Runs in**, in the **On this deployment** group:

| Option | Meaning |
|---|---|
| Size | `small` (1 vCPU, 2 GiB), `medium` (2 vCPU, 4 GiB) or `large` (4 vCPU, 8 GiB). Requests equal limits, so a sandbox gets exactly its size. The deployment decides which sizes it offers |
| Image | Any image with the sandbox guest. Empty uses the deployment's image. The deployment may limit images to a list of prefixes |
| Workspace path | Where commands and relative paths start, and where the workspace volume is mounted (default `/workspace`) |
| Desktop | Starts the sandbox from the desktop image and gives the agent the `computer` tool |
| Outbound network | Open, only listed IP ranges, or blocked |

In the API:

```json
{
  "target": {
    "kind": "managed",
    "provider": "kubernetes",
    "credential": { "source": "platform" },
    "options": { "size": "medium" }
  },
  "durability": "provider_snapshot"
}
```

## Pause and resume

When a session goes idle, Everruns deletes the pod and keeps the workspace volume: compute stops,
files stay, running processes end. The next command starts a new pod on the same volume. If the
volume itself is gone, Everruns starts a fresh sandbox. A per-organization limit counts running
sandboxes only, so paused ones never block new work.

## Network

Egress is enforced by the cluster's network policy, outside the sandbox, so code running as root
inside it cannot bypass it:

- **Open**: through the egress proxy (`egress: proxy`), which applies its deny lists and limits,
  or straight to the internet with private ranges excluded (`egress: direct`)
- **Only listed IP ranges**: the listed CIDR ranges and nothing else
- **Blocked**: nothing

A network policy cannot match domain names, so domain allowlists are refused when a template is
saved rather than silently widened. Your CNI must enforce NetworkPolicy (EKS with the VPC CNI
network policy agent, Calico, Cilium and recent kind releases do).

## Images

Everruns publishes two images with every release:

- `ghcr.io/everruns/sandbox-dev`: Microsoft's Dev Container Ubuntu 24.04 base with Node.js,
  Python with uv, Go, Rust, the Docker CLI, the GitHub CLI, ripgrep and jq
- `ghcr.io/everruns/sandbox-desktop`: the same, plus Xvfb, an xfce session, xdotool, screenshot
  tools, ffmpeg and Chromium

To use your own image, add the sandbox guest with the Dev Container Feature and allow the image
on your deployment:

```json
{
  "image": "mcr.microsoft.com/devcontainers/python:3",
  "features": {
    "ghcr.io/everruns/features/sandbox-guest:1": { "version": "v0.53.0" }
  }
}
```

The feature installs a static binary at `/usr/local/bin/everruns-sandbox-guest`, so it works on
any Linux base. The pod starts it itself; the image's own entrypoint is not used. Pin the guest
to the release your server runs.
