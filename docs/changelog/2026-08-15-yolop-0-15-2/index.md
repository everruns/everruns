---
title: "Yolop 0.15.2 release"
description: "Yolop 0.15.2 was released on 2026-08-15 with 6 highlights and 1 changelog entry. The release notes and any upgrade notes are linked here."
date: 2026-08-15
kind: release
significance: minor
products: [yolop]
release: "0.15.2"
---

## Highlights

- OpenRouter can be authenticated from the browser: `--provider openrouter` runs a PKCE login instead of requiring a hand-pasted key.
- Terra and Luna model variants join the OpenAI and Codex selectors, listed ahead of Sol.
- Malformed tool calls no longer reach execution, every provider's model-authored arguments are validated against the authoritative JSON Schema first, and `progress_checkpoint` stays callable when the progress gate demands it.
- ACP recovers provider authentication mid-session rather than stalling on an expired credential.
- The tmux gallery render is now gated on every PR by a real terminal implementation, so the release spec's terminal matrix reserves the human walk for GUI emulators.
- Tracing output no longer scribbles over terminal rows the TUI owns.

## In the changelog

- [OpenRouter connects with a browser sign-in](/changelog/2026-08-14-yolop-openrouter-browser-login/)

See the [GitHub release](https://github.com/everruns/yolop/releases/tag/v0.15.2) and the [full release notes](https://github.com/everruns/yolop/blob/main/CHANGELOG.md#0152---2026-08-15) for every pull request in this version.
