# GitHub Integration

The GitHub integration contributes three capabilities: `github`, with host-facing pull request tools; `github_scout`, which exposes no host tools and contributes a GitHub-backed agent blueprint (it depends on the built-in `subagents` capability); and `repositories`, the set of repositories a session works on in its sandbox.

## `github` capability

- `get_github_pull_request`, `get_github_pull_request_diff` (read-only) and `upsert_github_comment` (writes one comment per pull request or issue and marker).
- A managed comment starts with `<!-- everruns:<marker> -->`; the tool edits the first comment starting with that line, otherwise posts a new one. Markers are limited to letters, digits, `-`, `_`, `.`.
- `submit_github_pull_request_review` submits one review in a single request (no pending review is left behind). Events are `COMMENT` and `REQUEST_CHANGES` only. The review body starts with `<!-- everruns:review:<head sha> -->` and each inline comment with `<!-- everruns:finding:<key> -->`; a bot-authored review for the same commit makes the call a no-op, and keys a bot already posted are dropped. Inline comments must land on lines of the file's patch (added or context lines on `RIGHT`, removed or context lines on `LEFT`); others move into the review body.
- `upsert_github_issue` keys an issue by `<!-- everruns:finding:<fingerprint> -->` at the start of a bot-authored issue body among the newest 1,000 issues: open is updated, closed is left alone, missing is created. With `private_issues_only` it refuses public repositories.
- `create_github_pull_request` exists only with `allow_pull_requests: true`. Head must be a branch of the same repository; draft by default; an open pull request from the same head is returned instead of a second one.
- Only `Bot`-authored comments, reviews and issues are matched against markers, so a person cannot suppress a finding by pasting a marker.
- Tools act as the identity in `acts_as` (`agent` default, `person`, `person_or_agent`) through `get_github_token`, and each call's token is limited to the one repository it names and that tool's permissions (read tools `pull_requests: read`; comment `issues` + `pull_requests: write`; review `pull_requests: write`; issue `issues: write`; pull request `pull_requests: write` + `contents: read`).
- Arguments are validated (`owner/repo`, positive number) before any token is resolved or request sent.
- Diffs are capped on a UTF-8 boundary; the result reports total size and truncation.

## `repositories` capability

- Config: `repositories` (`repo`, optional `ref`, `access` `read` or `write`; at most 20, no duplicates), `add_repositories` (`off`, `read` default, `write`), `branch_prefix`, `depth`, `commit_name`, `commit_email`, `acts_as`.
- `repo_status` clones listed repositories that are missing, then lists every git checkout directly under the workspace root. `repo_add` exists unless `add_repositories` is `off`; `repo_push` exists only when a listed repository is writable or `add_repositories` is `write`.
- Layout: `/workspace/<name>`, else `/workspace/<owner>-<name>` when the name holds another repository or other files. Clones are shallow (`depth`), record the starting branch as `everruns.base`, and writable ones start on the work branch `<branch_prefix><last 8 hex of the session id>`.
- Tokens: one per operation through `get_github_token`, delivered with `exec_with_secret_file`. Clone and fetch ask `contents: read` for the repositories of one owner and require `permits_clone`; push asks `contents: write` for one repository and requires `permits_push`. Without installation-mint evidence nothing is delivered.
- `repo_push` authorizes by repository name against the config, requires the checkout to be on the work branch, pushes `HEAD` to `refs/heads/<work branch>` at `https://github.com/<repo>.git` (never the checkout's `origin`), and never forces.
- The credential helper answers only for host `github.com`; git hooks are disabled on these commands.
- A repository's `AGENTS.md` (first 16 KiB) is returned with its first checkout.

## Blueprints

### `github_scout`

Read-only scout agent for repository exploration.

- Searches GitHub code.
- Reads specific repository files by path and ref.
- Searches issues and pull requests through GitHub search qualifiers.
- Acts as the person when they connected GitHub, else the agent (`user_or_service`), with an unnarrowed token since it searches across repositories. If no token is available, tools return `connection_required`.
- Runs with fixed model `claude-haiku-4-5-20251001`.

## Security

- Scout tools are read-only and private to blueprint-backed child sessions. Host writes are limited to marked comments, reviews that cannot approve, marked issues, and (opt-in) draft pull requests.
- HTTP calls enforce the session network access policy for `https://api.github.com/` when present.
- Tokens are resolved through `get_github_token` or the `GITHUB_TOKEN` session secret fallback and are never returned in tool output. The `repositories` tools never use the session secret fallback.

## Tests

- Unit and mocked HTTP coverage: `cargo test -p everruns-integrations --features github`
- Live GitHub smoke test: `doppler run -- cargo test -p everruns-integrations --features github-live-tests --test live_api_test -- --test-threads=1`
