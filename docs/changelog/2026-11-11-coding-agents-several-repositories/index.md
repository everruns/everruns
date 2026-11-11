---
title: Coding agents that work across several GitHub repositories
description: Give an agent a set of repositories, some writable and some read-only. Each session gets one work branch, pushes it, and opens the pull requests.
date: 2026-11-11
kind: feature
significance: notable
products: [platform, cloud]
release: "0.52.0"
surfaces: [ui, api]
links:
  docs: [/capabilities/repositories/, /capabilities/code-navigation/]
  prs: [everruns/everruns#5307, everruns/everruns#5311, everruns/everruns#5329]
---

A change that renames a function in a library and fixes its callers in an app is now one session.
Add the [Repositories](/capabilities/repositories/) capability, or start from the beta `coding`
harness, and list what the agent works on:

```json
{
  "repositories": [
    {
      "repo": "acme/lib",
      "access": "write"
    },
    {
      "repo": "acme/app",
      "access": "write"
    },
    {
      "repo": "acme/docs"
    }
  ]
}
```

The session checks them out side by side in its sandbox, commits on one work branch per session,
pushes with `repo_push` and opens a pull request per repository. A push to a read-only repository
is refused. The agent's GitHub App token is minted per operation and never stays in the sandbox.
[Code navigation](/capabilities/code-navigation/) gives it a repository map, structural search and
language-server rename.

**In the UI:** pick repositories when you start a chat, and review every diff on the session's
**Changes** tab.

![The Changes tab of a session: two repositories on the work branch, with the files and diff of each](./changes@2x.png)
