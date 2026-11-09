# everruns-coding

Code navigation for [Everruns](https://everruns.com) agents: language servers,
structural search and edit, and repository maps.

| Capability | Tools | What it is for |
|---|---|---|
| `lsp` | `lsp_diagnostics`, `lsp_definition`, `lsp_references`, `lsp_hover`, `lsp_rename`, `lsp_symbols`, `lsp_code_actions` | Semantic answers from real language servers: where a symbol is defined, who uses it, renaming it everywhere |
| `ast_grep` | `ast_grep` | Read-only structural search with ast-grep patterns |
| `ast_edit` | `ast_edit` | Pattern rewrites, previewed before they are applied |
| `repo_map` | `repo_map`, `repo_symbols` | A compact symbol map for finding your way around unfamiliar code |

## Where the code lives

The tools do not assume the code is on the machine they run on. Each capability
is built over a `CodeHost`:

- `SandboxCodeHost`: the code is in the session's sandbox. Source files are
  fetched for parsing, edits are written back, and language servers run inside
  the sandbox. A hosted Everruns deployment uses this one.
- `LocalCodeHost` (feature `local`): the code is a directory on this machine,
  read directly, with language servers as child processes. A terminal agent
  such as [Yolop](https://github.com/everruns/yolop) uses this one.

```rust
use everruns_coding::{LspCapability, SandboxCodeHost};
use everruns_contracts::runtime::capabilities::Capability;
use std::sync::Arc;

let capability = LspCapability::new(Arc::new(SandboxCodeHost));
assert_eq!(capability.tools().len(), 7);
```

## Language servers

`lsp` knows rust-analyzer, typescript-language-server, pyright, gopls and
clangd, and starts one per project and language the first time a file of that
language is queried. The server binary has to be installed where the code
lives; when it is not, the tool says which one is missing. Other servers, or
other binaries for the built-in ones, are set in the capability config:

```json
{ "servers": { "zig": { "command": "zls", "extensions": ["zig"] } } }
```

A workspace that holds several repositories gets a server per repository,
rooted at that repository.

## Features

- `local`: `LocalCodeHost`, for code on the machine the agent runs on.
