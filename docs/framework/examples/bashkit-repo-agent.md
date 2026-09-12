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

## The important part

```rust
let agent = Agent::builder()
    .name("bashkit-repo-agent")
    // Tell the agent what the sandbox is, so it reaches for a portable
    // alternative instead of retrying a builtin Bashkit does not implement.
    .instructions("You are a release engineer for the repository mounted at \
        /workspace. The bash tool is your only way to see or change anything. \
        It is a sandboxed Bash interpreter, so there is no network, no host \
        filesystem, and no git; work with the files in the working tree.")
    .provider(OpenAI::from_env()?)
    .model("gpt-5.6-terra")
    // One real host directory becomes the session's `/workspace`. Every shell
    // path resolves under it — nothing above it is addressable.
    .workspace(&working_copy)
    // Writing is an explicit opt-in; the default policy is read-only.
    .workspace_policy(WorkspacePolicy::read_write())
    // The only capability: a sandboxed shell over that workspace. No file
    // tools, so every read and edit in the transcript is a bash command.
    .capability(BashkitShell::new())
    .build()?;

let session = Engine::new().create(agent);
// The example streams session events to print each shell script and its
// output; `session.send_and_wait(request)` is the version without the observer.
session.send_and_wait(&release_request(&release_date)).await?;

// The turn is not the result — the directory is. The host re-reads the working
// copy and checks each claim, so a confident summary over an unchanged tree
// fails loudly.
for check in verify(&working_copy, &release_date) {
    println!("[{}] {}", if check.passed { "ok" } else { "FAIL" }, check.label);
}
```
