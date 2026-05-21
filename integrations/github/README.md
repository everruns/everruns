# everruns-integrations-github

GitHub-backed agent blueprints for Everruns.

This crate registers the `github_scout` capability through Everruns' integration plugin system. The capability is blueprint-only: it does not expose GitHub tools to the host agent. Instead, it contributes the `github_scout` blueprint, whose private tools run only inside blueprint-backed child sessions.

## Blueprint

`github_scout` is a read-only scout agent for repository exploration.

- Searches code across GitHub repositories.
- Reads UTF-8 files by repository, path, and optional ref.
- Searches issues and pull requests with GitHub search qualifiers.
- Resolves credentials from the existing `github` user connection.
- Returns `connection_required` when no GitHub connection is available.
- Depends on the built-in `subagents` capability so hosts that enable `github_scout` also get `spawn_subagent`, `get_subagents`, and `message_subagent`.

## Tool Privacy

The host agent sees only the blueprint through Everruns' subagent spawning flow. The GitHub REST tools are instantiated inside the child session and are not added to the host tool list.

## Publishing

This crate is included in the Everruns crates.io publish workflow. Keep `Cargo.toml`, this README, and crate-level rustdoc current when changing the public crate surface.
