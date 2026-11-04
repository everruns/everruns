---
title: Tool events expose scoped worker CPU observations
description: Native hosts report worker-thread CPU consumed while polling a tool executor, separately from elapsed time and remote or child-process work.
date: 2026-10-14
kind: feature
significance: notable
products: [platform, framework]
release: "0.49.0"
surfaces: [api, code]
links:
  docs: [/event-reference/]
---

Read `worker_thread_cpu_ns` on `tool.completed` events to inspect native worker
CPU separately from elapsed tool duration:

```json
{
  "type": "tool.completed",
  "data": {
    "tool_call_id": "call_1",
    "tool_name": "read_file",
    "success": true,
    "status": "success",
    "duration_ms": 250,
    "worker_thread_cpu_ns": 1800000
  }
}
```

Linux and macOS hosts measure CPU around each executor-future poll, excluding
async suspension. Clock-bracketing overhead is included; child processes,
separately spawned or blocking work, and remote compute are outside the scope.
An absent field means unobserved, not zero. See the [Event Reference](/event-reference/)
for the observation contract.
