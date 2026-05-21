# GitHub Integration

The GitHub integration contributes the `github_scout` capability. It does not expose host-facing tools by default; it contributes a GitHub-backed agent blueprint and depends on the built-in `subagents` capability.

## Blueprints

### `github_scout`

Read-only scout agent for repository exploration.

- Searches GitHub code.
- Reads specific repository files by path and ref.
- Searches issues and pull requests through GitHub search qualifiers.
- Uses the existing `github` user connection. If no token is available, tools return `connection_required`.
- Runs with fixed model `claude-haiku-4-5-20251001`.

## Security

- Tools are read-only and private to blueprint-backed child sessions.
- HTTP calls enforce the session network access policy for `https://api.github.com/` when present.
- Tokens are resolved from user connections or the `GITHUB_TOKEN` session secret fallback and are never returned in tool output.

## Tests

- Unit and mocked HTTP coverage: `cargo test -p everruns-integrations-github`
- Live GitHub smoke test: `doppler run -- cargo test -p everruns-integrations-github --features github-live-tests --test live_api_test -- --test-threads=1`
