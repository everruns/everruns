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

## Platform gate

A deployment that charges for sandboxes installs a `PlatformSandboxGate`
(`kubernetes_sandbox::gate`) once at startup. Every pod start, create and
resume alike, asks it first; its refusal is the plain-words error the agent
sees. The grant's lifetime becomes the pod's `activeDeadlineSeconds`, so
Kubernetes itself bounds what a run can cost even if no Everruns process
comes back. The grant id and run id live as pod annotations, so whoever
deletes the pod (pause, delete, orphan cleanup) settles it with the time the
pod existed, capped at the lifetime, without any other state. A failed
settle is logged, never fatal: the gate owner counts an unsettled grant as
its whole lifetime. No gate means no limits beyond the deployment settings.
`KUBERNETES_SANDBOX_BILLING_NOTE` tells template authors what a run costs.

## Egress

Enforced by NetworkPolicy outside the guest, so root inside cannot bypass it:

- Open: only the tenant egress proxy (SOCKS, credential minted per sandbox
  and written to the exec env) plus DNS, or the internet minus private,
  link-local and CGNAT ranges (`direct`). The credential needs only the
  proxy secret: with no public SOCKS listener the host mints in-cluster
  credentials (empty address), which E2B treats as no route.
- IP allowlist: those CIDRs only, minus the same private ranges; a range
  that is itself private is refused. Blocked: nothing.
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
  platform gate admit/settle, orphan scan).
- `tests/kubernetes_sandbox_live.rs` on kind
  (`.github/workflows/kubernetes-sandbox-live.yml`): real pods, PVCs and
  network policy enforcement.

## Left out

Where the hosted demo runs sandboxes and how it meters demo credit are SaaS
decisions, made through the platform gate and deployment settings. No BYOK cluster connections: a user's own
cluster is not a supported credential.
