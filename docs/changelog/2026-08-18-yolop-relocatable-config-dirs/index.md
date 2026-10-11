---
title: "Config and data directories can be relocated"
description: "New --config-dir and --data-dir flags, or YOLOP_CONFIG_DIR and YOLOP_DATA_DIR, move Yolop's global settings and session data without changing HOME."
date: 2026-08-18
kind: feature
significance: minor
products: [yolop]
surfaces: [cli]
release: "0.17.0"
links:
  docs: ["https://github.com/everruns/yolop#cli-essentials"]
  prs: [everruns/yolop#600]
---

Yolop's global settings directory and its data directory can now be moved with
`--config-dir` and `--data-dir`, or with the `YOLOP_CONFIG_DIR` and
`YOLOP_DATA_DIR` environment variables. The value names Yolop's own directory,
so nothing appends another `yolop` folder, and a relative path resolves against
the directory Yolop started in.

```bash
yolop --config-dir ./tmp/yolop-config --data-dir ./tmp/yolop-data
```

Isolated runs, or several Yolop identities on one machine, no longer need a
different `HOME`.

See [CLI essentials](https://github.com/everruns/yolop#cli-essentials) and [#600](https://github.com/everruns/yolop/pull/600).
