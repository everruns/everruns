---
title: "Yolop extension management moves to the yolop extensions CLI"
description: "Model-callable extension management tools are removed from Yolop; install, enable, disable and reload extensions with the yolop extensions command instead."
date: 2026-08-17
kind: breaking
significance: notable
products: [yolop]
surfaces: [cli, agent]
release: "0.16.0"
links:
  docs: ["https://github.com/everruns/yolop/blob/main/docs/extensions.md"]
  prs: [everruns/yolop#587]
---

Yolop no longer gives the model tools to install, enable, disable or reload
extensions. Use `yolop extensions` instead: `list`, `install`, `remove`,
`enable`, `disable`, `reload`, `doctor`, `secret` and `scaffold`. Run from a
session's own shell, `enable`, `disable` and `reload` act on that running
session. Run from another terminal, they change global settings, and `reload`
is rejected because there is no session to restart.

```bash
yolop extensions install crates.io:yolop-extension-lsp
```

`/extensions`, and the commands and tools that installed extensions contribute,
are unchanged.

See [Extensions](https://github.com/everruns/yolop/blob/main/docs/extensions.md) and [#587](https://github.com/everruns/yolop/pull/587).
