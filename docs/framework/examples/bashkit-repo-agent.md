---
title: Bashkit Repo Agent
description: Cut a release in a real repository with the sandboxed Bashkit shell as the agent's only execution runtime.
---

The [Bashkit Repo Agent source](https://github.com/everruns/everruns/tree/main/examples/bashkit-repo-agent)
performs a repository operation end to end with `gpt-5.6-terra`: it bumps the
version in every crate manifest, folds the unreleased changelog fragments into a
new dated section, and deletes the fragments.

Every step runs through the [Bashkit Shell](/capabilities/bashkit-shell/)
capability, the agent's only tool. A throwaway working copy of the bundled
sample repository is mounted as `/workspace` with a read-write workspace policy,
so the shell edits real files on disk while staying inside the sandbox: no
subprocess, no host filesystem, no network, and no `git`.

![Bashkit Repo Agent terminal demo](https://raw.githubusercontent.com/everruns/everruns/main/examples/bashkit-repo-agent/demo.gif)

This screencast replays a real run in readable pages, with waiting time removed.
[Read the complete displayed transcript](https://github.com/everruns/everruns/blob/main/examples/bashkit-repo-agent/demo.txt).
Scripts and command output are truncated for display; the agent receives the
full result.

```bash
OPENAI_API_KEY=... cargo run -p everruns-bashkit-repo-agent
```

Pass a directory after `--` to keep the working copy after the run. When the
turn ends, the host re-reads the directory and prints one check per claim the
agent made, exiting non-zero if the release did not land. That check list is
what makes the example a test of the execution runtime rather than of the
model's summary.
