---
title: Shell commands can rename workspace directories
description: Shell mv and embedded Python os.rename now preserve nested files and empty directories when renaming workspace folders through remote workers.
date: 2026-10-29
kind: fix
significance: notable
products: [platform, framework, cloud]
cloud: 2026-10-30
surfaces: [agent, code]
links:
  docs: [/capabilities/bashkit-shell/]
  prs: [everruns/everruns#5072]
---

[Bashkit Shell](/capabilities/bashkit-shell/) can rename workspace directories
through the server-backed filesystem. Shell `mv` and embedded Python `os.rename`
previously failed with a file-not-found error when the source was a directory.

Moving a staged folder now preserves its nested files, empty directories and
file identities. The destination must be absent. Readonly trees, protected
parents and filesystem mount boundaries remain protected. Custom filesystem
backends must opt in to atomic directory moves before these commands can use
them.
