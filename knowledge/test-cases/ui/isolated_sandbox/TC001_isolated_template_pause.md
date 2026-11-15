---
type: Test Case
title: "TC001: Isolated Sandbox Template - Pause Keeps Files"
description: "Verify that an Isolated sandbox template runs a session in its own pod, refuses a domain allowlist, warns without a VM runtime, and keeps workspace files across an idle pause."
tags:
  - everruns
  - test-case
  - ui
  - isolated-sandbox
---
# TC001: Isolated Sandbox Template - Pause Keeps Files

## Description

Verify the Isolated sandbox target end to end on a Helm install: the picker
groups it under "On this deployment", the editor shows sizes from the
deployment and the no-VM warning, a session runs in its own pod, and an idle
pause keeps the workspace while the pod goes away.

## Preconditions

- Helm install on kind with `sandboxes.kubernetes.enabled=true`,
  `egressProxy.enabled=true`, no `runtimeClassName`
- `kubectl` access to the cluster, LLM provider configured

## Test Data

| Field | Value |
|-------|-------|
| Template | Runs in **Isolated sandbox**, Size **Medium · 2 vCPU · 4 GiB**, Outbound network **Open** |
| Idle | Pause after 60 seconds |
| Message 1 | Run `echo kept > notes.txt && nproc && curl -s -o /dev/null -w '%{http_code}' https://example.com` |
| Message 2 | Run `cat notes.txt` |

## Steps

1. Sandboxes → Templates → New. Open **Runs in**: verify **Isolated sandbox**
   is under **On this deployment** and Daytona, E2B, boxd under **Your
   provider account**.
2. Pick **Isolated sandbox**. Verify no Provider account field, the
   **Not isolated: plain containers** warning, and Containment **Container
   only, no VM isolation**.
3. Set Outbound network to **Only listed IP ranges**, enter `example.com`,
   save: verify the save is refused with a message about domain allowlists.
4. Use the test data instead and save. Attach to an agent, start a session,
   send Message 1.
5. Verify `nproc` prints `2` and `example.com` answers `200` (through the
   egress proxy). `kubectl -n everruns-sandboxes get pods,pvc,networkpolicy`
   shows one `er-sbx-...` of each.
6. Open Sandboxes → Fleet, click the sandbox: verify Provider **Isolated
   sandbox**, Isolation **Container only, no VM isolation**, Size **Medium**.
7. Wait for the idle pause: the pod is gone, the PVC stays, the fleet shows it
   paused.
8. Send Message 2.

## Expected Result

- Message 2 prints `kept`; a new pod started on the same volume.
- Deleting the session removes the pod, PVC, Secret and NetworkPolicy.
