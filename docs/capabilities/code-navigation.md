---
title: Code Navigation
description: Language servers, structural search and edit, and repository maps for agents that work on code in a sandbox.
appliesTo: [framework, platform, cloud]
---

Code navigation gives an agent the tools a developer's editor has: jump to a
definition, find every use of a symbol, rename it everywhere, search code by
its structure, and get a symbol map of an unfamiliar repository. Text search
answers "where does this string appear". These answer "where is this symbol
used", which text search gets wrong for same-named symbols, aliases and
re-exports.

It is four capabilities. Add the ones the agent needs:

| Capability | ID | Tools |
|---|---|---|
| [Language servers](#language-servers) | `lsp` | `lsp_diagnostics`, `lsp_definition`, `lsp_references`, `lsp_hover`, `lsp_rename`, `lsp_symbols`, `lsp_code_actions` |
| [Structural search](#structural-search) | `ast_grep` | `ast_grep` |
| [Structural edit](#structural-edit) | `ast_edit` | `ast_edit` |
| [Repository map](#repository-map) | `repo_map` | `repo_map`, `repo_symbols` |

All four work on the code in the session's sandbox, under `/workspace`. The
session needs a sandbox: without one, each tool answers with an error that says
so. Paths in arguments and results are relative to `/workspace`, so a workspace
that holds several repositories names files as `my-app/src/main.rs`.

## Language servers

`lsp` runs real language servers and asks them the question. It knows these
out of the box:

| Language | Server | Files |
|---|---|---|
| Rust | `rust-analyzer` | `.rs` |
| TypeScript and JavaScript | `typescript-language-server` | `.ts`, `.tsx`, `.js`, `.jsx`, `.mts`, `.cts`, `.mjs`, `.cjs` |
| Python | `pyright-langserver` | `.py`, `.pyi` |
| Go | `gopls` | `.go` |
| C and C++ | `clangd` | `.c`, `.h`, `.cc`, `.cpp`, `.cxx`, `.hpp`, `.hh` |

The server binary has to be installed in the sandbox image. When it is not, the
tool names the missing binary, and the agent can install it and call again.

A server starts the first time a file of its language is queried, so the first
call is slower while the project is indexed. It then stays up inside the
sandbox and later calls reuse it. In a workspace with several repositories each
repository gets its own server, started in that repository's root (the nearest
directory with a `.git`).

| Tool | What it answers |
|---|---|
| `lsp_diagnostics` | Errors and warnings in one file, without running a build |
| `lsp_definition` | Where the symbol at a position is defined (also type definition, implementation, declaration) |
| `lsp_references` | Every use of the symbol at a position, across the project |
| `lsp_hover` | Type signature and documentation at a position |
| `lsp_rename` | Renames a symbol everywhere it is used; `dry_run` previews the edits |
| `lsp_symbols` | The outline of a file, or a fuzzy symbol search across the project |
| `lsp_code_actions` | Quick fixes and refactorings at a position, and applies one by title |

Positions are 1-based `line` and `column`, counted in characters.

`lsp_rename` and `lsp_code_actions` change files. An edit is refused when it
would touch anything outside the workspace or inside `.git`, `node_modules`,
`target` and other vendored or build directories.

### Configuration

```json
{
  "ref": "lsp",
  "config": {
    "servers": {
      "python": { "command": "basedpyright-langserver", "args": ["--stdio"] },
      "zig": { "command": "zls", "extensions": ["zig"] },
      "go": { "enabled": false }
    },
    "request_timeout_ms": 60000,
    "diagnostics_wait_ms": 15000
  }
}
```

| Field | Default | Meaning |
|---|---|---|
| `servers.<key>.command` | built-in | Executable to run for this server |
| `servers.<key>.args` | built-in | Its arguments |
| `servers.<key>.extensions` | built-in | File extensions routed to it |
| `servers.<key>.enabled` | `true` | `false` turns a built-in server off |
| `request_timeout_ms` | `60000` | How long one request to a server may take |
| `diagnostics_wait_ms` | `15000` | How long `lsp_diagnostics` waits for fresh results |

A key that is not built in (`zig` above) adds a server and needs both `command`
and `extensions`.

## Structural search

`ast_grep` searches code by shape with [ast-grep](https://ast-grep.github.io/)
patterns. `$NAME` matches one node and `$$$ARGS` matches any number:

```json
{
  "name": "ast_grep",
  "arguments": {
    "pattern": "console.log($$$ARGS)",
    "language": "typescript",
    "path": "my-app/src"
  }
}
```

It reads Rust, Python, TypeScript, TSX, JavaScript, Go, C#, CSS, HTML and Bash.
Each match comes back with its file, position, text and captured variables.

## Structural edit

`ast_edit` rewrites every match of a pattern with a replacement that can use the
captured variables. It previews by default: the first call returns a diff and
changes nothing. Calling again with `dry_run: false` applies it.

```json
{
  "name": "ast_edit",
  "arguments": {
    "pattern": "console.log($$$ARGS)",
    "replacement": "logger.info($$$ARGS)",
    "language": "typescript",
    "dry_run": false
  }
}
```

## Repository map

`repo_map` returns a compact map of the symbols in the workspace or a
subdirectory, grouped by file: functions, types, methods and their signatures.
It is the first thing to call in unfamiliar code. `repo_symbols` returns the
same symbols as a flat list ranked against a `query`.

Both read Rust, Python, JavaScript, TypeScript, C#, Go, Zig, CSS, HTML, Bash,
SQL, Java, C, C++, PHP, Ruby, Kotlin and Scala.

## Limits

Structural search, structural edit and repository maps read the source files
out of the sandbox for each call. One call covers up to 20,000 source files and
24 MiB of compressed source. Past that the tool asks for a narrower `path`.
Files larger than 1 MiB and vendored or build directories (`.git`,
`node_modules`, `target`, `dist`, `build`, `.venv` and similar) are skipped.

## See Also

- [Daytona](/capabilities/daytona/), [E2B](/capabilities/e2b/) and
  [Container Sandbox](/capabilities/container-sandbox/), the sandboxes these
  tools work in
- [GitHub](/capabilities/github/), for pull requests on the code
