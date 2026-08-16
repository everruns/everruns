# everruns-integrations-github

> GitHub-backed agent blueprints for Everruns.

[![Crates.io](https://img.shields.io/crates/v/everruns-integrations-github.svg)](https://crates.io/crates/everruns-integrations-github)
[![Documentation](https://docs.rs/everruns-integrations-github/badge.svg)](https://docs.rs/everruns-integrations-github)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-integrations-github` registers the `github_scout` capability through
the Everruns integration plugin system. The capability is blueprint-only: it does
not expose GitHub tools to the host agent. Instead, it contributes the
`github_scout` blueprint, a read-only repository-exploration scout whose private
tools run only inside blueprint-backed child sessions.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents. It registers with
[`everruns-core`](https://crates.io/crates/everruns-core) through the Everruns
integration plugin system.

## Quick Example

```rust
use everruns_core::capabilities::Capability;
use everruns_integrations_github::GitHubScoutCapability;

let capability = GitHubScoutCapability;

assert_eq!(capability.id(), "github_scout");
```

## What It Provides

### The `github_scout` Blueprint

A read-only scout agent for repository exploration:

- Searches code across GitHub repositories.
- Reads UTF-8 files by repository, path, and optional ref.
- Searches issues and pull requests with GitHub search qualifiers.
- Resolves credentials from the existing `github` user connection.
- Returns `connection_required` when no GitHub connection is available.
- Depends on the built-in `subagents` capability, so hosts that enable
  `github_scout` also get `spawn_agent` with `target.type: "subagent"`.
  Monitoring and steering use the generic `session_tasks` tools
  (`list_tasks`, `get_task`, `message_task`, `cancel_task`).

## Tool Privacy

The host agent sees only the blueprint, through Everruns' subagent spawning flow.
The GitHub REST tools are instantiated inside the child session and are never
added to the host tool list.

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-integrations-github)
- [GitHub Scout capability](https://docs.everruns.com/capabilities/github-scout/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
