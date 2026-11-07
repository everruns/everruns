---
title: "Opt-in apply_patch editing for OpenAI models"
description: "The native_edit_tools capability gives OpenAI models in Yolop the Codex apply_patch tool, and every yolop subcommand now has a description and an example."
date: 2026-10-09
kind: feature
significance: minor
products: [yolop]
surfaces: [cli, agent]
release: "0.20.0"
links:
  docs: ["https://github.com/everruns/yolop/blob/main/docs/features/tools.md"]
  prs: [everruns/yolop#827, everruns/yolop#922]
---

A new opt-in capability, `native_edit_tools`, gives OpenAI models Codex's
`apply_patch` tool. Claude models keep `edit_file`, which already matches their
`str_replace` semantics, and the default file-edit tools stay available. Enable
it like any capability, in settings or a profile:

```toml
[[capabilities]]
ref = "native_edit_tools"
```

Every `yolop` subcommand's help now has a description and a worked example, and
the system prompt prefers well-known CLIs such as `git` and `gh` over
improvised scripts.

See [Tools](https://github.com/everruns/yolop/blob/main/docs/features/tools.md) and [#827](https://github.com/everruns/yolop/pull/827).

## Update 2026-11-07

Native `apply_patch` now preserves the original bytes of untouched lines in files
with mixed LF and CRLF endings, including context matched with whitespace
flexibility. A final line without a newline stays unterminated when untouched,
and moving a file without editing it keeps its bytes unchanged. Added lines use
CRLF when the original file contains CRLF, otherwise LF. Appending after an
unterminated line adds the separator needed for the new line. See
[#922](https://github.com/everruns/yolop/pull/922).
