---
title: "Bashkit 0.18.2 release"
description: "sed is rewritten to follow GNU sed 4.9 so scripts written for Linux behave the same, Windows paths resolve correctly, and releases are hardened."
date: 2026-09-22
kind: release
significance: minor
products: [bashkit]
release: "0.18.2"
---

## Highlights

- **`sed` is GNU-aligned.** The stream editor was rewritten after a differential audit against GNU sed 4.9 found 46 mismatches. It adds the missing commands (`y = l n N P D T z F r R w W`), GNU range-address semantics, `\U \L \u \l \E` case conversion, a real getopt (`-s`, `-z`, `-f`, `-l`, attached `-i.bak`), and it preserves a missing final newline. `-i` now writes atomically ([#2436](https://github.com/everruns/bashkit/pull/2436)).
- **Windows path fix.** VFS paths use forward slashes on every host ([#2435](https://github.com/everruns/bashkit/pull/2435)).
- **Release pipeline hardening.** npm publish verification now outlasts the registry cache ([#2426](https://github.com/everruns/bashkit/pull/2426)), and untrusted workflow code no longer receives the checkout token ([#2420](https://github.com/everruns/bashkit/pull/2420)).

## In the changelog

- [sed is rewritten to follow GNU sed 4.9](/changelog/2026-09-21-bashkit-gnu-aligned-sed/)

See the [GitHub release](https://github.com/everruns/bashkit/releases/tag/v0.18.2) and the [full release notes](https://github.com/everruns/bashkit/blob/main/CHANGELOG.md#0182---2026-09-22) for every pull request in this version.
