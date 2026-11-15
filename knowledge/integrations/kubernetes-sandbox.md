---
type: Specification
title: "Isolated Sandboxes (Kubernetes)"
description: "The kubernetes managed Sandbox Template target: one pod per sandbox on the deployment's own cluster, the provider-neutral sandbox guest, VM isolation by runtime class, network-enforced egress, images, and what is left out."
tags:
  - everruns
  - integrations
  - sandboxes
---
# Isolated Sandboxes (Kubernetes)

## Why

Every other managed target needs a provider account (Daytona, E2B, Modal,
boxd). A self-hosted deployment already runs Kubernetes, and EKS now runs a pod
in its own VM (AWS Nitro Sandboxes, runtime class `isolated-sandbox`); gVisor
and Kata do the same elsewhere. So the deployment can run real sandboxes itself:
no account, no per-provider egress hook, the same Helm chart.

## Shape

- **Target** `managed` / provider `kubernetes`, shown as "Isolated sandbox".
  Credential source `platform`: the deployment's own cluster access, never a
  user connection. On when the server reaches Kubernetes and
  `KUBERNETES_SANDBOX_NAMESPACE` is set; no feature flag.
- **One pod per sandbox** (`er-sbx-<hex>`) with a workspace PVC, a Secret
  (guest token, exec env) and a NetworkPolicy, all labelled with the session's
  managed labels plus `everruns.sandbox`. Server and workers hold a Role in the
  sandbox namespace only (chart: `templates/sandboxes.yaml`).
- **Sizes** small / medium / large, requests equal limits. Per-organization
  cap counts running pods only.
- **Durability** `provider_snapshot`. Pause deletes the pod and keeps the
  PVC: files survive, processes do not (Nitro cannot pause with memory).
  Resume re-applies Secret and NetworkPolicy (the policy may have changed, the
  proxy credential expires) and starts a new pod. Missing PVC means Lost: a
  fresh sandbox replaces it. An evicted pod is replaced on resume.
- **Cleanup** joins the leased-resource and orphan-scan paths: the worker
  deletes by id, the orphan scan lists across organizations by label.

Code: `crates/integrations/src/kubernetes_sandbox/` (provider, pod spec,
settings), Helm `sandboxes.kubernetes`, server validation in
`domains/sandbox_templates/` (`target_options.rs`, `resolution.rs`).

## Sandbox guest

Pods have no exec API Everruns can rely on (the API server's exec is a
streaming protocol that bypasses no policy and needs `pods/exec`), so the image
runs a small agent: `everruns-sandbox-guest`, HTTP+JSON on port 8080.

- Protocol types in `everruns-contracts` (`sandbox_guest`), server and client
  in `everruns-integrations` (`sandbox_guest`), binary
  `src/bin/everruns-sandbox-guest.rs`. No new crate.
- Provider-neutral: any target whose platform starts a container but offers
  no exec or file API can reuse it.
- Auth is a per-sandbox token in `x-everruns-guest-token`, not
  `Authorization`, which the API server's pod proxy consumes. The token is
  stripped from command environments.
- Commands run in their own process group and are killed on timeout. Output,
  file and timeout caps live in the contract.
- Two routes: pod IP (server in the cluster) or the API server's pod proxy
  (server outside, tests, `kind`).

## Isolation

The runtime class is a deployment setting, so the template cannot weaken it.
Without one the pods are plain containers; the target descriptor reports
`isolation.vm_isolated = false` and the template editor warns that this is no
boundary for untrusted code. Outside a VM the container also drops
privilege escalation and `NET_RAW`. The sandbox namespace enforces Pod
Security `baseline`.

## Egress

Enforced by NetworkPolicy outside the guest, so root inside cannot bypass it:

- Open: only the tenant egress proxy (SOCKS, credential minted per sandbox
  and written to the exec env) plus DNS, or the internet minus private,
  link-local and CGNAT ranges (`direct`).
- IP allowlist: those CIDRs only. Blocked: nothing.
- Domain allowlists are refused at template save and at creation: a
  NetworkPolicy cannot match names, and widening silently would be worse.

Not used: Nitro's network-isolated pod annotation, which would also cut the
server off from the guest.

Known gaps: in proxy mode DNS is open (an exfiltration channel); a sandbox
running past the proxy credential's two-day lifetime without a pause loses
egress (never falls back to direct); domain allowlists wait for proxy-side
enforcement.

## Images

`ghcr.io/everruns/sandbox-dev` (Microsoft Dev Container Ubuntu base plus
Node, Python/uv, Go, Rust, Docker CLI, gh, ripgrep, jq) and
`sandbox-desktop` (adds Xvfb, xfce, xdotool, screenshot tools, ffmpeg,
Playwright's Chromium). Tagged `v<version>` like every release image, which is
what `default_image()` names. The guest is a static musl binary, also shipped
as the Dev Container Feature `ghcr.io/everruns/features/sandbox-guest`, so
any Linux image can become a sandbox image. Sources: `docker/sandbox/`.

## Desktop

`target.options.desktop` picks the desktop image, sets `DISPLAY`, starts the
display when the pod is ready, and serves `open_desktop` through the shared X
session in `x11_desktop.rs` (also used by E2B): xdotool and the display
scripts run as guest argv exec, never a shell line.

## Tests

- `tests/sandbox_guest.rs`: the guest end to end (exec, timeouts, files,
  archives, auth).
- `tests/kubernetes_sandbox.rs`: the provider against an in-process
  Kubernetes API whose pods run the real guest (lifecycle, eviction, lost
  volume, egress policies, caps, image limits, failed pulls, API proxy route,
  orphan scan).
- `tests/kubernetes_sandbox_live.rs` on kind
  (`.github/workflows/kubernetes-sandbox-live.yml`): real pods, PVCs and
  network policy enforcement.

## Left out

Hosted placement on the demo (separate cluster vs prod node pool) and demo
credit metering are SaaS decisions. No BYOK cluster connections: a user's own
cluster is not a supported credential.
