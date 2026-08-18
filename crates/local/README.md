# everruns-local

> Compatibility facade for the Everruns local in-process backend.

The local backend — SQLite-backed session catalog, task and schedule state, git
workspaces, and `LocalConfig` — was absorbed into the `everruns` facade crate in
0.19 and now lives at `everruns::local`. This crate re-exports it so existing
`everruns-local` dependants keep compiling against `everruns-host` 0.19 instead
of resolving the old, now-incompatible 0.18 package.

It is part of the [Everruns](https://everruns.com) ecosystem. New code should
depend on `everruns` directly rather than on this facade.

## Quick start

```rust
use everruns_local::LocalProfile;

// Re-exported from `everruns::local`.
let profile = LocalProfile::new("./agent-data");
```

Prefer depending on the facade crate directly for new code:

```toml
everruns = { version = "0.18", default-features = false, features = ["local"] }
```

## What It Provides

- Re-exports every public item of `everruns::local` (`LocalConfig`,
  `LocalProfile`, `SqliteDb`, `LocalGitWorkspaceProvider`, the runtime builder,
  schedule/session stores, and more) under the original `everruns_local::` path
- A drop-in upgrade path to `everruns-host` 0.19 for existing dependants
- No behavior of its own — it is a thin, source-compatible facade

## Documentation

- [Framework persistence](https://docs.everruns.com/framework/persistence/)
- [Workspaces and environments](https://docs.everruns.com/framework/workspaces-and-environments/)
- [API reference](https://docs.rs/everruns-local)

## License

[MIT](https://github.com/everruns/everruns/blob/main/LICENSE)
