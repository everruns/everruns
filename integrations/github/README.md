# everruns-integrations-github

> GitHub pull request tools and agent blueprints for Everruns.

[![Crates.io](https://img.shields.io/crates/v/everruns-integrations-github.svg)](https://crates.io/crates/everruns-integrations-github)
[![Documentation](https://docs.rs/everruns-integrations-github/badge.svg)](https://docs.rs/everruns-integrations-github)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-integrations-github` registers two capabilities through the Everruns
integration plugin system:

- `github`: host-facing tools for one pull request at a time (read it, read its
  size-capped diff, review it with inline comments, keep one managed comment on
  it up to date) and for filing deduplicated findings as issues. This is what a
  pull request reviewer or a scheduled security scanner uses.
- `github_scout`: blueprint-only. It contributes the `github_scout` blueprint, a
  read-only repository-exploration scout whose private tools run only inside
  blueprint-backed child sessions.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents. It registers with
[`everruns-core`](https://crates.io/crates/everruns-core) through the Everruns
integration plugin system.

## Quick Example

```rust
use everruns_core::capabilities::Capability;
use everruns_integrations_github::{GitHubCapability, GitHubScoutCapability};

assert_eq!(GitHubCapability.id(), "github");
assert_eq!(GitHubCapability.tools().len(), 5);
assert_eq!(GitHubScoutCapability.id(), "github_scout");
```

## What It Provides

### The `github` Capability

- `get_github_pull_request`: title, description, author, state, branches,
  size, and up to 100 changed files.
- `get_github_pull_request_diff`: the unified diff, cut to a byte budget
  (default 60,000, at most 200,000) with a `truncated` flag.
- `upsert_github_comment`: posts a comment, or edits the one this tool posted
  earlier under the same hidden marker, so an agent run on every push keeps a
  single summary comment current.

- `submit_github_pull_request_review`: one review (`COMMENT` or
  `REQUEST_CHANGES`, never approve) with inline comments. Comments the agent
  already posted on the pull request (same finding key) are skipped, comments
  on lines outside the diff move into the review body, and a second review of
  the same head commit is a no-op.
- `upsert_github_issue`: files a finding as an issue keyed by a fingerprint.
  An open issue for the same fingerprint is updated, a closed one is left
  closed.
- `create_github_pull_request` (only with `allow_pull_requests: true`): opens a
  draft pull request from a branch already pushed to the same repository.

Capability config (`GitHubConfig`), both off by default:

- `allow_pull_requests`: offer `create_github_pull_request`.
- `private_issues_only`: `upsert_github_issue` refuses public repositories.

All of them authenticate as the session's `github` connection. For an agent with
its own GitHub App, that is the App's installation, so comments appear as the
agent's bot.

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
