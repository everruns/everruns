---
title: "Yolop Linux sandbox blocks asynchronous socket paths"
description: "Linux sandboxed shell modes deny io_uring entry syscalls while preserving direct socket rules, Unix sockets and synchronous toolchain operations."
date: 2026-10-13
kind: fix
significance: notable
products: [yolop]
surfaces: [cli, operations]
links:
  prs: [everruns/yolop#850]
---

Linux shell containment now denies all three io_uring entry syscalls in
`read-only` and `workspace-write` mode, closing an asynchronous path outside
the direct socket filter. Internet and packet socket rules remain in place;
direct Unix sockets and ordinary synchronous toolchain operations remain
available.

Enable workspace containment through the existing setting:

```toml
sandbox_mode = "workspace-write"
```

Programs requiring io_uring need a synchronous fallback. Host reads, local Unix
sockets and shared temporary files retain their documented limits. See
[Yolop shell sandboxing](https://github.com/everruns/yolop/blob/main/docs/features/sandboxing/sandboxing.md)
for modes and platform requirements.
