# GitHub Integration

The GitHub integration contributes two capabilities: `github`, with host-facing pull request tools, and `github_scout`, which exposes no host tools and contributes a GitHub-backed agent blueprint (it depends on the built-in `subagents` capability).

## `github` capability

- `get_github_pull_request`, `get_github_pull_request_diff` (read-only) and `upsert_github_comment` (writes one comment per pull request or issue and marker).
- A managed comment starts with `<!-- everruns:<marker> -->`; the tool edits the first comment starting with that line, otherwise posts a new one. Markers are limited to letters, digits, `-`, `_`, `.`.
- Arguments are validated (`owner/repo`, positive number) before any token is resolved or request sent.
- Diffs are capped on a UTF-8 boundary; the result reports total size and truncation.

## Blueprints

### `github_scout`

Read-only scout agent for repository exploration.

- Searches GitHub code.
- Reads specific repository files by path and ref.
- Searches issues and pull requests through GitHub search qualifiers.
- Uses the existing `github` user connection. If no token is available, tools return `connection_required`.
- Runs with fixed model `claude-haiku-4-5-20251001`.

## Security

- Scout tools are read-only and private to blueprint-backed child sessions. `upsert_github_comment` is the only write, limited to issue comments carrying its marker.
- HTTP calls enforce the session network access policy for `https://api.github.com/` when present.
- Tokens are resolved from user connections or the `GITHUB_TOKEN` session secret fallback and are never returned in tool output.

## Tests

- Unit and mocked HTTP coverage: `cargo test -p everruns-integrations-github`
- Live GitHub smoke test: `doppler run -- cargo test -p everruns-integrations-github --features github-live-tests --test live_api_test -- --test-threads=1`
