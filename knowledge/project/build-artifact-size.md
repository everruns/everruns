---
type: Specification
title: "Build Artifact Size"
description: "Why the Everruns library crates compile large, and which levers actually move the number."
tags:
  - everruns
  - project
  - build
---
# Build Artifact Size

## Abstract

The three largest workspace library crates produce debug rlibs an order of magnitude
larger than their sources, which repeatedly prompts "what is bloating these crates?".
This concept records the measured breakdown so the question does not have to be
re-investigated, and separates the levers that work from the ones that look plausible
and do not.

## Measured Baseline

Dev profile, default features, `rustc` 1.96.0:

| Crate | rlib | `lib.rmeta` | objects | `.text` |
|---|---|---|---|---|
| `everruns-core` | 32.9 MB | 14 MB | 18 MB | 3.8 MB |
| `everruns-platform` | 23.6 MB | 10 MB | 13 MB | ~2.5 MB |
| `everruns-provider` | 20.7 MB | 8 MB | 12 MB | ~2.5 MB |

Enabling `openapi` adds ~3 MB to `everruns-core`.

Section totals across core's 256 object files: `.strtab` 5.2 MB, `.text` 3.8 MB,
`.rela*` 2.6 MB, `.symtab` 1.6 MB, plus ~3 MB of section headers for 48,443 sections.

Two facts follow, and they are the whole answer:

1. **Executable code is ~10% of the file.** The bulk is crate metadata plus ELF
   symbol/relocation bookkeeping for mangled names of monomorphized generics.
2. **Size tracks API surface, not waste.** Nothing oversized is embedded: `include_str!`
   covers three small schema files, the workspace `docs/` embed is behind a non-default
   feature, and published `.crate` tarballs carry 145/68/34 files.

Serde derives account for 41% of core's `.text` (1.57 MB of 3.83 MB), from 232
`Serialize` plus 230 `Deserialize` derives. All 282 `ToSchema` sites are already
`cfg_attr`-gated behind `openapi`.

## Constraints

- Profile knobs are already at their useful settings: `debug = 0` on both `dev` and
  `test`, `openapi` off by default, CI and pre-push building with `CARGO_INCREMENTAL=0`.
  Treat the profile block as tuned and do not re-litigate it without new measurements.
- Incremental state, not artifacts, dominates local disk. Building core, platform, and
  provider alone produced 2.6 GB of `target/debug/incremental` against 2.0 GB of
  `target/debug/deps`. Reclaim it with `just prune`, which keeps compiled artifacts.
- Incremental compilation stays on for local development: a warm edit rebuild of
  `everruns-core` measured ~4 s against ~10 s with `CARGO_INCREMENTAL=0`. The 2.5x
  inner-loop cost is not worth the disk outside CI and pre-push.
- Narrowing public API surface does not shrink artifacts, and was measured rather than
  assumed. 222 of core's 989 named public items are referenced nowhere outside the crate;
  demoting the 158 of them that are not root re-exports moved the rlib from 32.9 MB to
  32.7 MB and `lib.rmeta` from 13.74 MB to 13.71 MB. Metadata is encoded for the crate's
  own use regardless of visibility, so `pub(crate)` buys API hygiene, not bytes. Do not
  fund a visibility audit as a size measure.
- Demotion does expose dead code that `pub` masks: the same experiment produced 95
  `never used` warnings inside core. Deleting genuinely dead code is the one lever that
  removes both metadata and object bytes, and it is worth doing on its own merits.

## Success Bar

- A claim that a crate is "too big" cites a measured breakdown, not the rlib size alone.
- Changes made for size report before/after numbers from the same profile and features,
  built the same way: incremental and non-incremental builds of identical sources produce
  rlibs that differ by enough (32.9 MB against 33.5 MB for core) to invent a result.

Relevant references:
- [`knowledge/project/maintenance.md`](maintenance.md) - artifact size as a maintained property.
- [`knowledge/project/dismissed-options.md`](dismissed-options.md) - `codegen-units` tuning.
- [`scripts/lib/prune-build-cache.sh`](../../scripts/lib/prune-build-cache.sh) - reclaims incremental caches.
- [`crates/core/public-api.txt`](../../crates/core/public-api.txt) - core's tracked public surface.
