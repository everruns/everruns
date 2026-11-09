---
type: Specification
title: "Code Navigation"
description: "Language servers, ast-grep and repository maps as hosted capabilities that work on code inside a session's sandbox, shared with Yolop through the everruns-coding crate."
tags:
  - everruns
  - harnesses
  - coding
  - sandbox
---
# Code Navigation

`lsp`, `ast_grep`, `ast_edit` and `repo_map` give an agent editor-grade
understanding of code. They are the first step of the hosted coding harness:
the harness itself (repository sets, credentials, prompt) builds on them. The
public description is `docs/capabilities/code-navigation.md`; the source is
`crates/coding/`.

## Decisions

**One implementation, shared with Yolop.** The tools began in Yolop. Everruns
cannot depend on the `yolop` crate, because Yolop pins `everruns-*` at one exact
version and would pull in a second copy of core. So the code moved here, and
Yolop uses it from here after a release. Nothing is copied.

**The Everruns loop drives the sandbox; no agent runs inside it.** Installing a
Yolop binary in the sandbox, or running Yolop there over ACP, was rejected: the
agent loop, its budget, approvals and history stay in Everruns. The only thing
that runs in the sandbox is the code's own tooling, a language server included.

**A separate crate, `everruns-coding`.** This is an exception to the rule of
fewer crates. The tree-sitter grammars are a heavy build, and only hosts that
want coding tools should pay for it. It is in `.github/crates-publish-set.txt`.

**Where the code lives is one trait.** `CodeHost` has two implementations:

- `LocalCodeHost` (feature `local`): a directory on this machine, language
  servers as child processes. Yolop's case.
- `SandboxCodeHost`: the session's sandbox, reached through the
  `SandboxWorkspace` tool-context extension.

The parsing code and the LSP client are the same for both.

## How the sandbox host works

A sandbox offers three operations: run one command and get its output, read a
file, write a file. There is no open pipe and no shared disk. Everything is
built from those three, and needs only a shell and coreutils in the image.

- **Parsing runs in the worker.** One command archives the matching source
  files, the worker reads the archive back in parts under the provider's
  per-read cap, and unpacks it into a private temporary directory that is
  removed after the call. The archive is untrusted input: only regular files
  with in-tree relative paths are kept, under fixed size budgets.
- **Edits go back as one batch**, then the workspace is checkpointed once, not
  once per file.
- **A language server runs detached inside the sandbox**, with standard input
  on a FIFO and standard output appended to a file. While a tool call is using
  the server, the worker moves bytes between that pair and its LSP client with
  short commands. Between calls nothing runs on the worker's side and the
  server waits.

The alternative for language servers, a `socat` listener per server, was
dropped: it needs a package the image may not have and an open port inside the
sandbox.

## Contracts

- **Several repositories from the start.** A language server is keyed by
  project root and language. The project root of a file is the nearest
  directory holding `.git`, so a workspace with several checkouts gets one
  server per repository, rooted where its build files are. Paths in arguments
  and results are relative to the workspace root.
- **No state on the worker that must survive.** The table of running servers
  is a cache. After a worker restart, or when another worker picks up the
  session, the next call starts a server again and replaces the stale process
  in the sandbox. A workspace's servers are dropped after 30 minutes without
  use.
- **A server proposes, the host disposes.** Edits from `lsp_rename` and
  `lsp_code_actions` are worked out in memory against the current files and
  refused when any target is outside the workspace or inside a vendored or
  build directory. The host checks each target again when it applies the
  change; the local host also resolves symlinks there.
- **A sandbox's own answer passes through.** When the sandbox answers with
  something other than an error string (a connection prompt, an internal
  error), the tool returns that result unchanged.
- **Servers are on by default and start lazily.** A missing server binary is
  reported by name so the agent can install it.

## Tests

`crates/coding` runs the whole sandbox host against a real shell: a fake
sandbox executes each command with `sh` in a temporary directory, so the
archive, upload, file-operation and relay scripts are exercised as written. The
LSP tools run against an in-process fake server.
