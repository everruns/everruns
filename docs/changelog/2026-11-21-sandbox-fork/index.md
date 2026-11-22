---
title: Forks and subagents can copy the Sandbox
description: A forked Session or a subagent can start from a copy of the source's running Sandbox, with packages and build output already in place, on E2B and Daytona VMs.
date: 2026-11-21
kind: feature
significance: minor
products: [platform, cloud]
surfaces: [api, agent]
links:
  docs: [/features/sandbox-templates/]
---

Forking a Session used to hand the fork an empty Sandbox, so it had to
reinstall dependencies and rebuild before it could try anything. Now a fork can
take a copy of the parent's running Sandbox instead:

```bash
curl -X POST http://localhost:9300/api/v1/sessions/{session_id}/fork \
  -H "Authorization: Bearer $EVERRUNS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"sandbox": "fork"}'
```

Agents get the same option. `spawn_agent` with `"sandbox": "fork"` starts a
subagent on a copy of the parent's machine, so three subagents can try three
fixes on the same warmed-up checkout without stepping on each other.

On E2B the copy includes memory and running processes. On Daytona it works for
VM Sandboxes. Anywhere else the new Session starts fresh, exactly as before. See
[Copy a Sandbox into a fork or subagent](/features/sandbox-templates/#copy-a-sandbox-into-a-fork-or-subagent).
