---
title: "Set up as a Buzz Desktop harness in one command"
description: "The yolop into buzz command writes a custom harness file for Buzz Desktop, so Buzz can start Yolop over ACP without anyone editing harness JSON by hand."
date: 2026-08-15
kind: feature
significance: minor
products: [yolop]
surfaces: [cli]
release: "0.16.0"
links:
  docs: ["https://github.com/everruns/yolop#in-your-editor-over-acp"]
  prs: [everruns/yolop#579]
---

`yolop into buzz` writes a custom harness file for Buzz Desktop, next to the
existing `yolop into zed` and `yolop into paseo` commands. Buzz then starts
Yolop over ACP.

```bash
yolop into buzz
```

Running the command again keeps your `env` values and any extra fields in the
file; `--force` replaces the file completely.

See [In your editor, over ACP](https://github.com/everruns/yolop#in-your-editor-over-acp) and [#579](https://github.com/everruns/yolop/pull/579).
