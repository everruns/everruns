---
title: "Yolop installs compiled extensions with prebuilt binaries"
description: "yolop extensions install now downloads a compiled extension's server binary for your platform and verifies its SHA-256 checksum before writing anything."
date: 2026-08-20
kind: feature
significance: notable
products: [yolop]
surfaces: [cli]
release: "0.17.0"
links:
  docs: ["https://github.com/everruns/yolop/blob/main/docs/extensions.md"]
  prs: [everruns/yolop#620]
---

Extensions whose server is a compiled program now install end to end. A package
can declare `capabilityServer.binaries`, a URL template over `{name}`,
`{version}` and `{target}`. When the server command is not already available,
`yolop extensions install` downloads the archive for your machine's target
triple and places the binary in the package's `bin/` directory.

The archive's `.sha256` file is required: an archive without one, or with a
mismatched checksum, is refused and nothing is written. `yolop-extension-logfire`
0.2.0 is the first extension published this way.

See [Extensions](https://github.com/everruns/yolop/blob/main/docs/extensions.md) and [#620](https://github.com/everruns/yolop/pull/620).
