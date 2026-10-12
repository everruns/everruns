---
title: "Check out private pull requests into the managed sandbox"
description: "The new github_checkout_pull_request tool fetches a pull request into the managed sandbox with a read-only token that never stays in the sandbox."
date: 2026-10-12
kind: feature
significance: minor
products: [platform, cloud]
surfaces: [agent]
links:
  docs: [/capabilities/github/]
---

A review agent with the [GitHub capability](/capabilities/github/) and a
managed sandbox can now check out a pull request from a private repository and
build, test or diff it with `bash`. Before, `git clone` in the sandbox only
worked for public repositories.

```json
{
  "repo": "acme/app",
  "number": 42
}
```

`github_checkout_pull_request` fetches the head and base branch into
`/workspace/app` and returns the commit hashes. The token it uses reads one
repository, is passed through a temporary file removed after the fetch, and
never lands in the git config, later shell commands or sandbox checkpoints.
