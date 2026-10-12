---
title: Host Shell gives Framework agents a kernel-contained Bash
description: The new host_shell capability lets an embedded Everruns agent run real host processes inside a Seatbelt or Landlock policy that fails closed.
date: 2026-09-19
kind: feature
significance: notable
products: [framework]
surfaces: [code]
release: "0.30.0"
links:
  docs: [/capabilities/host-shell/, /framework/examples/host-shell-agent/]
  prs: [everruns/everruns#3715]
---

Embedders can give an agent a `bash` tool that runs real child processes on the
host, with the kernel enforcing what it may touch: Seatbelt on macOS, Landlock
plus seccomp on Linux. The `host_shell` capability ships behind the `host-shell`
feature on the `everruns` or `everruns-host` crate, which the Everruns server does
not enable.

`HostCompute::contained(options)` builds the contained environment, and a
`ShellApprovalGate` decides whether each command runs at all; with no gate the
answer is no. A missing kernel primitive is a launch error, never a silent fall
back to an uncontained shell.

See [Host Shell](/capabilities/host-shell/) and the
[Host Shell Agent example](/framework/examples/host-shell-agent/). [#3715](https://github.com/everruns/everruns/pull/3715)
