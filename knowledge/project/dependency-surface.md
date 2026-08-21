---
type: Specification
title: "Dependency Surface"
description: "How the Rust dependency graph is measured, what was removed or reimplemented in-tree, and what deliberately stays."
tags:
  - everruns
  - project
  - build
  - dependencies
---
# Dependency Surface

## Abstract

"What can we drop or reimplement?" recurs whenever build times or the supply-chain
surface come up. This concept records how the graph is measured, the removals that were
made and their evidence, and the dependencies deliberately kept, so the question is
answered from measurement rather than re-argued from intuition. It is the dependency
counterpart to [Build Artifact Size](build-artifact-size.md), which owns compiled-size
levers.

## Method

Candidates are ranked by **exclusive transitive cost**: the crates that leave the graph
entirely if a dependency goes, computed as the closure of that dependency minus the
closure of every other workspace direct dependency. A crate shared with the rest of the
tree costs nothing to keep, however large it looks alone. The second axis is **call-site
surface**: how many distinct APIs the workspace actually touches, which is what a
reimplementation has to reproduce.

Two measurement traps produced wrong answers before they were caught, and both are easy
to repeat:

- **Sibling dependencies mask each other.** Measuring `ethers-signers` alone put its cost
  at 11 crates, because `ethers-core` was separately a direct dependency and its subtree
  counted as "shared". Measured together the real figure was 64. Always measure a
  cohesive stack as one target set.
- **Dev-dependencies are not shipped.** OpenSSL appeared to be in the server graph until
  the edges were restricted with `cargo tree -e normal`, which showed it entering only
  through `everruns-sdk`, a dev-dependency of the load-test bench. Check what ships
  before claiming a runtime dependency exists.

Wall-clock build deltas are *not* recorded here; claiming one requires a measured
before/after under the constraints in [Build Artifact Size](build-artifact-size.md).

## Current Shape

- **713 distinct third-party packages**, down from 827, with 62 packages resolving at
  two or more versions, down from 86.
- Only 7 direct dependencies are dev-only across the whole workspace, so nearly the
  entire graph ships.
- No shipped binary links OpenSSL. `cargo tree -p everruns-server -i openssl-sys -e
  normal` returns nothing; the remaining lock entries are reachable only through the
  bench's dev-dependency on the published SDK.

## What Was Replaced, and Why

Each of these traded a dependency for in-tree code that the workspace owns and tests.
The rule applied throughout: reimplement only where the contract is small, fully
specified, and testable against a fixed vector.

| Dependency | Replacement | Evidence it is equivalent |
|---|---|---|
| `ethers-signers`, `ethers-core` | [`domains/payments/eip712.rs`](../../crates/server/src/domains/payments/eip712.rs) on `k256` + `sha3` | A signature vector captured from the ethers implementation, pinned in `authority.rs`, passed unchanged across the swap |
| `metrics-exporter-prometheus` | [`api/prometheus_recorder.rs`](../../crates/server/src/api/prometheus_recorder.rs) | Exposition-format unit tests; see [Prometheus Metrics](../operations/prometheus-metrics.md) for the shape change |
| `git2`'s TLS backend | [`domains/git_fetch.rs`](../../crates/server/src/domains/git_fetch.rs), smart-HTTP v2 over the rustls client | Protocol unit tests plus an `#[ignore]`d end-to-end fetch against a real remote |
| `sysinfo` | [`everruns-durable::sysstat`](../../crates/durable/src/sysstat.rs), reading `/proc` | Unit tests on a Linux host; returns `None` elsewhere rather than a fabricated zero |
| `dirs`, `webbrowser` | `crates/cli/src/user_dirs.rs`, `crates/cli/src/browser.rs` | Resolved paths asserted identical; `credentials_path` is a stable on-disk location |
| `minijinja` | Single-pass substitution in the durable bench report | Placeholder-coverage test over the real template |
| `fetchkit`'s `render-rakers` | Removed; browserless and deno serve JS rendering | Product-contract change, recorded under TM-TOOL-024 |

Three of these deserve their reasoning kept:

### Payments signing is in-tree because both libraries were worse

`ethers-rs` is retired upstream, so the move was forced. Its maintained successor
`alloy-signer-local` was measured before adopting it and came out at **103 exclusive
crates against ethers' 64**, because it drags the consensus, network, RLP, and KZG stack
to sign one struct. The x402 rail needs exactly one EIP-712 struct over secp256k1, which
is fully specified; in-tree it costs 15 crates.

This is the one place the "don't own crypto" instinct was overridden, and it was
overridden on evidence: a known-answer vector proves byte-equivalence with the
implementation it replaced, and the module carries independent checks — signature
recovery to the signing address, low-S form, EIP-55 vectors from the specification, and
per-field digest separation. Any change there must keep that vector passing.

### git2 stays; only its transport left

libgit2 still backs `session_git`'s Postgres ODB and now indexes fetched packfiles, so
replacing it was never the goal. `gitoxide` was measured as an alternative and would
have cost **60 crates to remove git2's 5**. What was worth removing was the TLS backend:
`vendored-openssl` compiled OpenSSL from source on every image build and was the only
reason the builder stage installed perl and make.

### Rendering was a product call, not a cleanup

`render-rakers` linked a JavaScript engine and an HTML/CSS selector stack into every
binary carrying web-fetch, and was the only reason the workspace resolved a
`syn 0.15` / `quote 0.6` / `proc-macro2 0.4` generation. Dropping it removed the
`render` argument from the tool's schema — a contract change, decided as one.

## Remaining Candidates

- **`serde_yaml`** — zero exclusive cost, but upstream is archived. It parses skill and
  agent frontmatter in `crates/core/src/skill.rs`, `api/agents.rs`,
  `domains/knowledge_bases/okf.rs`, and the CLI. Options are a maintained fork or a
  purpose-built frontmatter parser; the parsed surface is small and typed. Supply-chain
  hygiene, not size.
- **`wiremock`** (dev-only, 6 crates) — used by 24 crates' tests. A minimal mock server
  in `everruns-test-support` would remove it, at the cost of reimplementing matchers.
- **`governor`** (4 crates) — two independent rate limiters in the server. If they are
  ever unified, a GCRA cell over the existing map is the natural implementation.
- **`jsonschema`** (18 crates, 2 call sites) — validates the plugin manifest and
  delegation results. Worth revisiting only if those schemas stay small and in-tree.

## Constraints

- Exclusive cost, not tree size, decides. `parking_lot` looks removable at 27 call sites,
  but `tokio`, `fred`, `moka`, and `object_store` all pull it. The same holds for `hex`,
  `base64`, and `futures-util`.
- `async-trait` stays: 343 call sites across 47 crates, over `dyn` traits that native
  `async fn` does not cover. `utoipa` stays at 557 sites, already `cfg_attr`-gated behind
  `openapi`.
- Correctness-critical dependencies are not reimplementation candidates regardless of
  cost: `aes-gcm`, `argon2`, `jsonwebtoken`, `rustls`, `sqlx`, `image` (decoders), `zip`
  (deflate), `rusqlite` ([session-sqldb](../runtime-resources/session-sqldb.md) is a
  product surface), and `bashkit`.
- Duplicate versions are mostly not ours to fix. `reqwest 0.12` alongside `0.13` comes
  from `everruns-sdk`, the `a2a-*-lf` crates, and `reqwest-eventsource`; `http 0.2` comes
  from the AWS SDK behind the Bedrock driver. Chase these by upgrading upstreams.

## Success Bar

- A removal proposal names the exclusive crate count it eliminates and the call sites it
  must reproduce, both recomputed from the current `Cargo.lock`, measuring cohesive
  stacks as one target set and distinguishing shipped from dev-only edges.
- A reimplementation lands with tests covering the behavior the dependency provided, and
  a comment at the implementation naming the crate it replaced and why.
- Anything replacing a correctness-critical dependency carries a fixed vector proving
  equivalence with what it replaced, and is security-reviewed before merge.
- Build-time or artifact-size claims cite measurements taken under the same profile and
  features, per [Build Artifact Size](build-artifact-size.md).
