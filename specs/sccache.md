# sccache — Shared Compile Cache

## Purpose

Speed up Rust builds by caching compiled artifacts in S3. Particularly valuable for:
- Cloud agent environments (cold target/ dir each session)
- CI jobs (shared cache across runners)
- Local dev after `cargo clean` or toolchain bumps

## Results

| Scenario | Without sccache | With sccache (warm) | Speedup |
|----------|----------------|---------------------|---------|
| Clean build (internal-protocol + deps) | ~2m 17s | ~56s | ~2.5x |
| Incremental (local crate only) | ~4s | ~4s | — |

Cache hit rate on warm cache: ~99.8%.

## Architecture

```
rustc invocation
  → RUSTC_WRAPPER=sccache
    → hash(source + flags + deps)
      → S3 lookup (s3://everruns-sccache/sccache/*)
        → hit: return cached .o
        → miss: compile, upload result
```

**Backend:** S3 bucket `everruns-sccache` in `us-east-1`. Credentials in Doppler (`SCCACHE_BUCKET`, `SCCACHE_REGION`, `SCCACHE_AWS_ACCESS_KEY_ID`, `SCCACHE_AWS_SECRET_ACCESS_KEY`).

**Fallback:** Without Doppler/S3 credentials, sccache uses local disk cache (`~/.cache/sccache`). Still useful for repeated builds.

## Requirements

- `CARGO_INCREMENTAL=0` — required; incremental compilation is incompatible with sccache.
- sccache binary in PATH.

## Files

| File | Purpose |
|------|---------|
| `scripts/lib/sccache.sh` | Install, configure, activate helper |
| `scripts/init-cloud-env.sh` | Auto-installs sccache during cloud init |
| `.github/scripts/ci-sccache-env.sh` | CI-specific S3 credential export |
| `.github/workflows/ci.yml` | Sets `RUSTC_WRAPPER: sccache` globally |

## Usage

### Cloud agent (recommended)

Installed automatically by `./scripts/init-cloud-env.sh`. Activate before builds:

```bash
source scripts/lib/sccache.sh && activate_sccache
cargo build  # uses sccache
```

Or wrap with Doppler for credential injection:

```bash
doppler run -- bash -c 'source scripts/lib/sccache.sh && activate_sccache && cargo build'
```

### Local development (optional)

```bash
just sccache-setup          # one-time install + configure
source scripts/lib/sccache.sh && activate_sccache
cargo build                 # cached
just sccache-stats          # verify hits
```

Local dev benefits most after `cargo clean`, toolchain updates, or switching branches with different dependency trees. For normal incremental builds, Cargo's built-in incremental compilation is already fast.

### CI

Already integrated. See `.github/workflows/ci.yml` (`RUSTC_WRAPPER: sccache`) and `.github/scripts/ci-sccache-env.sh`.

## Design Decisions

- **Optional, not forced:** sccache is never required. All builds work without it. The `activate_sccache` function returns non-zero if unavailable, letting callers continue.
- **S3 over Redis/memcached:** S3 is simpler (no server to run), durable, and already used in CI. Cost is negligible for compile artifacts.
- **Pre-built binary:** Avoids bootstrapping problem (can't use sccache to build sccache). Downloads ~15MB musl binary.
- **CARGO_INCREMENTAL=0:** Required by sccache. In cloud agents this is already set (incremental is wasteful for single builds). For local dev, the S3 cache hit compensates for lost incremental compilation on clean builds.
