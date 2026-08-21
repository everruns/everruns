---
type: Specification
title: "Dependency Surface"
description: "Which third-party crates are worth removing or reimplementing in-tree, ranked by measured transitive cost against call-site surface."
tags:
  - everruns
  - project
  - build
  - dependencies
---
# Dependency Surface

## Abstract

"What can we drop or reimplement?" recurs whenever build times or the supply-chain
surface come up. This concept records the measured shape of the Rust dependency graph,
the ranked candidates, and the ones deliberately kept, so the question is answered from
evidence rather than re-argued from intuition. It is the dependency counterpart to
[Build Artifact Size](build-artifact-size.md), which owns compiled-size levers.

## Method

Candidates are ranked by **exclusive transitive cost**: the number of crates that leave
the graph entirely if the direct dependency goes, computed as the reachable closure of
that dependency minus the closure of every other workspace direct dependency. A crate
shared with the rest of the tree costs nothing to keep, however large it looks in
isolation. The second axis is **call-site surface**: how many distinct APIs the
workspace actually touches, which is what a reimplementation has to reproduce.

Both are derived from `Cargo.lock` and the source tree. Wall-clock build deltas are
*not* recorded here; claiming one requires a measured before/after under the constraints
in [Build Artifact Size](build-artifact-size.md).

## Baseline

- 827 distinct third-party packages resolve for the workspace, from 129 direct
  dependencies across all workspace members.
- 86 packages resolve at two or more versions. `cargo deny` treats this as a warning
  (`bans.multiple-versions = "warn"` in `deny.toml`), not a failure.
- Only 7 direct dependencies are dev-only across the whole workspace
  (`a2a-server`, `http-body-util`, `insta`, `serde_html_form`, `trybuild`, `turmoil`,
  `wiremock`), so nearly the entire graph ships.

## Ranked Candidates

Exclusive cost is in crates removed from the resolved graph.

| Dependency | Excl. | Where | Verdict |
|---|---|---|---|
| `ethers-signers` / `ethers-core` | 11 | `payments/authority.rs`, 4 APIs | **Replace.** Upstream is retired. |
| `fetchkit`'s `render-rakers` | ~20 | `integrations/web-fetch` | **Decide.** Ships a JS engine for an opt-in feature. |
| `git2`'s `vendored-openssl` | 3 | 4 files in `crates/server` | **Replace the transport, keep libgit2.** |
| `metrics-exporter-prometheus` | 9 | `api/prometheus.rs`, 1 API | **Reimplement.** Text exposition is a small `Recorder`. |
| `sysinfo` | 9 | `durable` backpressure + 2 benches | **Reimplement.** Linux-only `/proc` read. |
| `dirs` | 5 | `cli`, `everruns/local`, 4 sites | **Reimplement.** XDG lookup is a few dozen lines. |
| `governor` | 4 | two rate limiters in `server` | **Reimplement or unify.** GCRA is small; the two limiters already duplicate policy. |
| `serde_yaml` | 0 | frontmatter in 5 files | **Replace for maintenance, not size.** |
| `wiremock` (dev) | 6 | 24 crates' tests | **Consider folding into `everruns-test-support`.** |
| `minijinja` | 2 | `durable/src/bench/report.rs` only | **Drop.** One bench report template. |
| `webbrowser` | 2 | `cli/commands/login.rs`, 1 call | **Drop.** |
| `dialoguer` | 2 | `cli`, 4 sites | **Keep unless the CLI already owns a prompt layer.** |
| `hex` | 0 | 53 sites | **Keep.** Zero exclusive cost; reimplementation buys nothing. |

### `ethers-*` — the clearest removal

Highest-value target and the only one where the current dependency is a liability
independent of size. `ethers-rs` is retired upstream, and it is the sole reason the
graph still carries `syn 1.x`-era crates: `thiserror 1.0`, `rand 0.8`, `uuid 0.8`,
`ethabi`, `coins-bip32`, `coins-bip39`, `eth-keystore`.

The whole surface is four APIs in one file
([`crates/server/src/domains/payments/authority.rs`](../../crates/server/src/domains/payments/authority.rs)):
`LocalWallet::from_str`, `Signer::address`, `Signer::sign_typed_data`, and
`ethers_core::utils::to_checksum`. Two directions:

1. **Migrate to `alloy-signer-local`.** Mechanical, maintained, and keeps EIP-712
   encoding in a library that is audited for it.
2. **Reimplement on `k256` + Keccak.** EIP-712 struct hashing and EIP-55 checksumming
   are fully specified and small, and the workspace already depends on the RustCrypto
   stack. This trades a maintained dependency for signing code we own.

Prefer (1). EIP-712 domain-separator and struct-encoding bugs are silent and produce
signatures a counterparty rejects or, worse, accepts over the wrong payload. Owning that
is not the kind of code this repository benefits from owning.

### `fetchkit` rendering — a product decision, not a cleanup

`fetchkit`'s `render-rakers` feature pulls `rakers` → `scraper` + `html5ever` +
`rquickjs` (a QuickJS binding) → `dlopen` → `dlopen_derive`, which is what drags
`syn 0.15`/`quote 0.6`/`proc-macro2 0.4` into the lock file. That is a JavaScript engine
and a full HTML/CSS selector stack linked into the server for a request-opt-in fetch
mode ([`integrations/web-fetch/src/lib.rs`](../../integrations/web-fetch/src/lib.rs),
`THREAT[TM-TOOL-024]`).

The workspace already ships browser-grade integrations (`browserless`, `deno`) that
overlap with in-process rendering. Whether `render=rakers` survives is a product call
about the fetch tool's contract; if it does not, dropping the feature removes ~20 crates
with no code to write. Reimplementing the non-rendering path (fetch, HTML-to-markdown,
Web Bot Auth signing) is a much larger job than it looks and is not recommended.

### `git2` — keep libgit2, drop OpenSSL

`git2` is used deeply: PostgreSQL-backed object storage via mempack ODB hydrate/drain,
revwalk, diff, and commit construction in
[`crates/server/src/domains/session_git/service.rs`](../../crates/server/src/domains/session_git/service.rs).
Reimplementing that is out of scope, and gitoxide would be a migration, not a deletion.

The cost worth attacking is narrower: `vendored-openssl` builds OpenSSL from source and
makes `git2` the only OpenSSL consumer in a tree that is otherwise entirely rustls
(`openssl-sys` reverse-deps are `git2` and `libgit2-sys` alone). Network use is confined
to clone/fetch in `knowledge_indexes/source_sync.rs` and `memory/source_sync.rs`. Moving
those two call sites onto a smart-HTTP-v2 fetch over the existing rustls `reqwest`
client, and letting libgit2 index the resulting packfile, removes the second TLS stack
and the from-source OpenSSL build.

### Small in-house replacements

These are individually minor; they matter as a group because each one also removes a
platform-specific subtree the workspace never exercises.

- **`sysinfo`** drags `windows`, `windows-collections`, `windows-future`, `objc2-io-kit`,
  and `ntapi` for CPU and memory sampling in `durable`'s worker backpressure and two
  benches. Server and worker deploy to Linux containers; a `/proc/stat` and
  `/proc/meminfo` reader covers the entire usage.
- **`dirs`** drags `redox_users`, `libredox`, and `option-ext` for `config_dir`,
  `data_local_dir`, and `home_dir`. XDG plus a macOS branch is a small module.
- **`governor`** brings `dashmap` and is used by two independent rate limiters
  (`auth/rate_limit.rs`, `api/channel_rate_limit.rs`). If those are ever unified, a GCRA
  cell over the existing map is the natural implementation.
- **`metrics-exporter-prometheus`** brings `loom`, `generator`, `evmap`, `left-right`,
  and `hashbag` behind a single `PrometheusBuilder`/`PrometheusHandle` call site. A
  `metrics::Recorder` that renders the text exposition format is well-bounded work.
- **`minijinja`** renders one bench report and should be `format!`.

### `serde_yaml` — a maintenance flag, not a size one

Zero exclusive cost, but upstream is archived. It parses skill and agent frontmatter in
`crates/core/src/skill.rs`, `crates/server/src/api/agents.rs`,
`crates/server/src/domains/knowledge_bases/okf.rs`, and the CLI. The realistic options
are a maintained fork or a purpose-built frontmatter parser; the parsed surface is small
and typed, which makes the second viable. Track it as supply-chain hygiene.

## Constraints

- Exclusive cost, not tree size, decides. `parking_lot` looks removable at 27 call sites,
  but `tokio`, `fred`, `moka`, and `object_store` all pull it, so removing our uses saves
  nothing. The same holds for `hex`, `base64`, and `futures-util`.
- `async-trait` stays. 343 call sites across 47 crates, and the runtime is built on
  `dyn` trait objects that native `async fn` in traits does not cover.
- `utoipa` stays. 557 sites, already `cfg_attr`-gated behind the `openapi` feature per
  [Build Artifact Size](build-artifact-size.md).
- Correctness-critical dependencies are not reimplementation candidates regardless of
  cost: `aes-gcm`, `argon2`, `jsonwebtoken`, `rustls`, `sqlx`, `image` (decoders),
  `zip` (deflate), `rusqlite` (`session_sqldb` is a product surface, see
  [session-sqldb](../runtime-resources/session-sqldb.md)), and `bashkit`.
- `mlua`, `tree-sitter*`, `object_store`, and `include_dir` are already optional or
  feature-gated; their cost is opt-in and does not need a removal argument.
- Duplicate versions are mostly not ours to fix. `reqwest 0.12` alongside `0.13` comes
  from `everruns-sdk`, `a2a-client-lf`, `a2a-server-lf`, and `reqwest-eventsource`;
  `http 0.2` comes from the AWS SDK used by the Bedrock driver. Chase these by upgrading
  upstreams, not by vendoring.

## Success Bar

- A removal proposal names the exclusive crate count it eliminates and the call sites it
  has to reproduce, both recomputed from the current `Cargo.lock`.
- A reimplementation lands with tests covering the behavior the dependency provided, and
  a comment at the implementation naming the crate it replaced and why.
- Nothing on the correctness-critical list is reimplemented without a security review.
- Build-time or artifact-size claims cite measurements taken under the same profile and
  features, per [Build Artifact Size](build-artifact-size.md).
