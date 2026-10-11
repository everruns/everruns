---
title: "History, completion and bash -i work like bash"
description: "Bashkit adds history and fc, complete and compgen, bind, prompt expansion and an interactive bash -i child shell, each with state kept per interpreter."
date: 2026-10-08
kind: feature
significance: notable
products: [bashkit]
surfaces: [code]
links:
  docs: ["https://github.com/everruns/bashkit/blob/main/crates/bashkit/docs/compatibility.md"]
  prs: [everruns/bashkit#2648]
---

Bashkit now implements bash's interactive builtins, with state kept per
interpreter:

- `history` and `fc` under `set -o history` or `bash -i`, with `HISTSIZE`,
  `HISTFILESIZE`, `histappend` and `HISTFILE` in the VFS. `fc` reruns commands
  but opens no editor.
- `complete`, `compgen` and `compopt` with all actions.
- `bind` with bash's default emacs bindings.
- `${x@P}` prompt decoding.
- A `bash -i` child shell with `PS1`, `PS2` and `PS4` defaults, `~/.bashrc` or
  `--rcfile`, and `--norc`.

The Oils spec pass rate went from 89.0% to 92.8%.

See the [compatibility scorecard](https://github.com/everruns/bashkit/blob/main/crates/bashkit/docs/compatibility.md) and [#2648](https://github.com/everruns/bashkit/pull/2648).
