---
title: "Yolop 0.17.1 release"
description: "Yolop 0.17.1 was released on 2026-08-27 with 6 highlights and 1 changelog entry. The release notes and any upgrade notes are linked here."
date: 2026-08-27
kind: release
significance: minor
products: [yolop]
release: "0.17.1"
---

## Highlights

- Releases ship two binaries per target. `yolop-<target>.tar.gz` is the default-features build the Homebrew formula points at, and no longer carries the local-inference engine; `yolop-<target>-metal.tar.gz` and `yolop-x86_64-unknown-linux-gnu-cuda.tar.gz` carry the engine with the backend that target can use. The Linux tarball drops from 29.9 MB to 18.2 MB. Extension servers are now built for every target too, not just the last one the matrix collapsed onto.
- The runtime recovers from failure loops instead of repeating them: consecutive equivalent invocation, missing-command, wrong-path, and usage failures on an unchanged workspace now require a different action or a bounded progress checkpoint, and a checkpoint is a real trajectory transition rather than a hook-only backstop.
- File discovery is batch-native. `read_many_files` is advertised eagerly, so a model can read independent known paths in one call even when the provider emits one tool call per turn, and `repo_map` keeps its compact parameter schema in the first-turn profile.
- Input routing is one ordered ownership table, so a paste while the sandbox-approval prompt owns the screen no longer lands in the composer behind it.
- ACP prompts continue once after the provider exhausts its stream-stall retries, replaying from the durable transcript so settled tools are not re-run.
- The everruns family moves to core 0.19 and provider 0.20, where reasoning is ordered `ContentPart::Reasoning` artifacts rather than a flat message field, and tuika to 0.11.0.

## In the changelog

- [Run models on your machine with the local provider](/changelog/2026-08-21-yolop-local-inference-provider/)

See the [GitHub release](https://github.com/everruns/yolop/releases/tag/v0.17.1) and the [full release notes](https://github.com/everruns/yolop/blob/main/CHANGELOG.md#0171---2026-08-27) for every pull request in this version.
