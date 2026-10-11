---
title: "Yolop runs models on your machine with the local provider"
description: "An experimental local provider runs a model inside the Yolop binary, with Metal and CUDA release builds and weights fetched by yolop weights pull."
date: 2026-08-21
kind: feature
significance: notable
products: [yolop]
surfaces: [cli]
release: "0.17.1"
links:
  docs: ["https://github.com/everruns/yolop#providers-and-models"]
  prs: [everruns/yolop#581, everruns/yolop#599, everruns/yolop#623]
---

`--provider local` runs a model inside the Yolop process: no server, no API key
and no network during a turn. The default model is Qwen3-30B-A3B-Instruct in Q4
GGUF. Fetch the weights first, since a turn never waits on a download:

```bash
yolop weights pull
yolop --provider local
```

The provider is experimental. The engine ships in separate release archives,
`yolop-<target>-metal.tar.gz` and `yolop-x86_64-unknown-linux-gnu-cuda.tar.gz`.
The default archive, which Homebrew installs, leaves it out, so the Linux
tarball dropped from 29.9 MB to 18.2 MB. From source, build with the `metal` or
`cuda` feature.

See [Providers and models](https://github.com/everruns/yolop#providers-and-models), [#581](https://github.com/everruns/yolop/pull/581), [#599](https://github.com/everruns/yolop/pull/599) and [#623](https://github.com/everruns/yolop/pull/623).
