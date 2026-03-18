---
title: Bashkit
description: Sandboxed virtual Bash interpreter for multi-tenant agent environments. Run shell commands safely with process isolation, filesystem limits, and timeouts.
---

[Bashkit](https://github.com/everruns/bashkit) is a virtual Bash interpreter written in Rust. It provides sandboxed, in-process execution with no real filesystem access by default — purpose-built for running untrusted bash scripts in multi-tenant agent environments.

## Why Bashkit?

Agents need shell access to be effective — installing packages, running builds, inspecting files. But spawning real bash processes in a shared environment creates isolation, security, and resource control problems. Bashkit solves this by interpreting bash in-process against a virtual filesystem, giving agents a full shell experience without host access.

## Core Capabilities

- **POSIX-compliant shell language** — variables, parameter expansion, command substitution, arithmetic, pipelines, redirections, control flow, functions, arrays, globs, here-documents
- **85 built-in commands** — core I/O (`echo`, `cat`, `printf`), navigation (`cd`, `ls`, `find`), text processing (`grep`, `sed`, `awk`, `jq`, `sort`), file operations (`mkdir`, `rm`, `cp`, `mv`), archives (`tar`, `gzip`), network (`curl`, `wget` with domain allowlist), and more
- **Virtual filesystem** — pluggable backends: `InMemoryFs`, `OverlayFs`, `MountableFs`
- **Resource limits** — configurable caps on command count, loop iterations, and function call depth
- **Network allowlist** — HTTP requests via `curl`/`wget` require explicit per-domain authorization
- **Async-native** — built on tokio

### Experimental Features

- **Git** — virtual git operations within the VFS (no host access)
- **Python** — embedded Monty interpreter (pure Rust, Python 3.12 compatible) with VFS bridging

## How Everruns Uses Bashkit

Everruns integrates bashkit as the execution backend for the **virtual bash** agent capability. When an agent runs shell commands, they execute inside bashkit rather than a real shell.

### Session Filesystem Bridge

Bashkit's pluggable filesystem trait lets Everruns bridge the interpreter directly to the session file store. Files created by other tools are immediately visible inside bash, and vice versa — no pre/post sync needed. Session files are mounted at `/workspace` in the bash environment.

- **Live file visibility** — files written by other tools during bash execution are immediately visible
- **No sync overhead** — eliminates pre/post execution sync of the entire filesystem
- **Memory efficiency** — files read on-demand instead of loading all into memory
- **Single source of truth** — consistent file state across all agent capabilities

### Resource Controls

Bashkit's execution limits map directly to Everruns' per-session resource constraints — preventing runaway scripts from consuming shared infrastructure. The network allowlist ensures agents can only reach explicitly authorized domains.

## Links

- [GitHub repository](https://github.com/everruns/bashkit)
