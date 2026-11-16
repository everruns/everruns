---
title: Agents can merge their own GitHub pull requests
description: A new GitHub setting lets an agent take a pull request out of draft and merge it once GitHub reports every check passed and reviews are in.
date: 2026-11-15
kind: feature
significance: notable
products: [platform, cloud, framework]
release: "0.53.0"
surfaces: [agent, api]
links:
  docs: [/capabilities/github/]
---

An agent that opens a pull request can now finish it. Turn on `allow_merge` in
the agent's GitHub capability:

```json
{
  "ref": "github",
  "config": {
    "allow_pull_requests": true,
    "allow_merge": true
  }
}
```

The agent gets `mark_github_pull_request_ready` and `merge_github_pull_request`.
GitHub stays the judge: a merge goes through only when the pull request has no
conflicts, required reviews are in, and no check is failing or still running,
and only at the commit the agent read. The setting is off by default, and
branch protection still applies. See
[Merge a pull request](/capabilities/github/#merge-a-pull-request).

A coding session that returns after Daytona deleted its stopped sandbox now
gets a fresh one instead of a "sandbox not found" error on every tool call.
