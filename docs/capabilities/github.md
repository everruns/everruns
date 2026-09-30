---
title: GitHub
description: Read GitHub pull requests and their diffs, and keep one comment per pull request up to date.
---

| | |
|---|---|
| **ID** | `github` |
| **Category** | Integrations |
| **Features** | None |
| **Dependencies** | None |

The GitHub capability gives an agent what it needs to work on one pull request:
read it, read its diff, and leave a comment it can keep editing. Pair it with a
GitHub trigger to build a pull request summarizer or reviewer.

## Tools

| Tool | What it does |
|---|---|
| `get_github_pull_request` | Title, description, author, state, branches, size, and up to 100 changed files. |
| `get_github_pull_request_diff` | The unified diff, cut to `max_bytes` (default 60,000, at most 200,000). `truncated` says whether it was cut. |
| `upsert_github_comment` | Posts a Markdown comment, or edits the one it posted earlier with the same `marker` (default `summary`). |

Every tool takes `repo` (`owner/repo`) and `number`.

`upsert_github_comment` starts the comment with a hidden
`<!-- everruns:<marker> -->` line and looks for it on the next run, so an agent
that runs on every push updates one comment instead of adding a new one each
time. Use a different marker for each kind of comment.

## Authentication

The tools use the session's `github` connection. When the agent has its own
GitHub App connected, that is the App's installation, so comments are posted as
the agent's bot and the tools reach only the repositories the App is installed
on. Without a connection the tools return `connection_required`, which asks the
user to connect GitHub.

The session's network access policy must allow `https://api.github.com/` when
one is set.
