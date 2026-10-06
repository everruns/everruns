# GitHub Integration

The GitHub integration contributes two capabilities: `github`, with host-facing pull request tools, and `github_scout`, which exposes no host tools and contributes a GitHub-backed agent blueprint (it depends on the built-in `subagents` capability).

## `github` capability

- `get_github_pull_request`, `get_github_pull_request_diff` (read-only) and `upsert_github_comment` (writes one comment per pull request or issue and marker).
- A managed comment starts with `<!-- everruns:<marker> -->`; the tool edits the first comment starting with that line, otherwise posts a new one. Markers are limited to letters, digits, `-`, `_`, `.`.
- `submit_github_pull_request_review` submits one review in a single request (no pending review is left behind). Events are `COMMENT` and `REQUEST_CHANGES` only. The review body starts with `<!-- everruns:review:<head sha> -->` and each inline comment with `<!-- everruns:finding:<key> -->`; a bot-authored review for the same commit makes the call a no-op, and keys a bot already posted are dropped. Inline comments must land on lines of the file's patch (added or context lines on `RIGHT`, removed or context lines on `LEFT`); others move into the review body.
- `upsert_github_issue` keys an issue by `<!-- everruns:finding:<fingerprint> -->` at the start of a bot-authored issue body among the newest 1,000 issues: open is updated, closed is left alone, missing is created. With `private_issues_only` it refuses public repositories.
- `create_github_pull_request` exists only with `allow_pull_requests: true`. Head must be a branch of the same repository; draft by default; an open pull request from the same head is returned instead of a second one.
- Only `Bot`-authored comments, reviews and issues are matched against markers, so a person cannot suppress a finding by pasting a marker.
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

- Scout tools are read-only and private to blueprint-backed child sessions. Host writes are limited to marked comments, reviews that cannot approve, marked issues, and (opt-in) draft pull requests.
- HTTP calls enforce the session network access policy for `https://api.github.com/` when present.
- Tokens are resolved from user connections or the `GITHUB_TOKEN` session secret fallback and are never returned in tool output.

## Tests

- Unit and mocked HTTP coverage: `cargo test -p everruns-integrations --features github`
- Live GitHub smoke test: `doppler run -- cargo test -p everruns-integrations --features github-live-tests --test live_api_test -- --test-threads=1`
