---
title: "Review agents on Bashkit can read the code around a change"
description: "github_checkout_pull_request now works without a managed sandbox: on Bashkit it copies the pull request's files into the session files."
date: 2026-11-02
kind: feature
significance: minor
products: [platform, cloud]
surfaces: [agent]
links:
  docs: [/capabilities/github/]
---

A review agent on the Bashkit Worker harness used to see only the diff.
`github_checkout_pull_request` from the [GitHub capability](/capabilities/github/) now copies the pull request head's text files
into `/workspace/<repo>` in the session files when there is no managed sandbox,
so the agent can read the project's `AGENTS.md`, callers and nearby tests with
`grep`, `rg` and `cat` before it comments. No Daytona, E2B or Modal account is
needed.

```json
{
  "repo": "acme/app",
  "number": 42,
  "paths": [
    "AGENTS.md",
    "src/billing"
  ]
}
```

The copy is a snapshot, not a git repository, and binary files and files over
1 MiB are skipped and listed. The GitHub token stays on the host. The built-in
PR Reviewer template now checks the pull request out before reviewing it.

Walking the copy is fast too: `find` and `ls -R` over a 3,700-file checkout
now finish in under a second instead of hitting the 30-second shell timeout.

Session files can also now hold names that start with `_`, such as
`__init__.py` and `__tests__/`; only the top-level `_` folder stays reserved.
