---
title: "Turn activity compacts into one summary row by default"
description: "Fullscreen Yolop turns now collapse narration and tool output into one summary row that Ctrl+O expands; pass --no-compact-work to keep the full transcript."
date: 2026-09-03
kind: improvement
significance: notable
products: [yolop]
surfaces: [cli]
release: "0.17.3"
links:
  docs: ["https://github.com/everruns/yolop#cli-essentials"]
  prs: [everruns/yolop#612, everruns/yolop#658]
---

In the fullscreen TUI, each running turn now shows one summary row instead of
appending every narration line and tool call to the transcript. Press `Ctrl+O`
to expand or collapse the details of the current or latest turn. Completed,
failed and cancelled turns get distinct markers, and the final answer stays a
separate entry below the summary.

To keep the expanded transcript, start Yolop with:

```bash
yolop --no-compact-work
```

The mode was opt-in with `--compact-work` in 0.17.0 and became the default in
0.17.3.

See [CLI essentials](https://github.com/everruns/yolop#cli-essentials), [#612](https://github.com/everruns/yolop/pull/612) and [#658](https://github.com/everruns/yolop/pull/658).
