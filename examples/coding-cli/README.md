# ercode — a minimal coding agent on the `everruns` library

`ercode` is a small terminal coding agent built **only** on the public
[`everruns`](../../crates/everruns) crate. It is the executable acceptance test
for the OSS library surface: it depends on `everruns` alone — never on
`everruns-core` or `everruns-host` — so it demonstrates the exact dependency
and import path an application uses to embed an agent.

It shows the whole public loop:

- build an agent with `everruns::Agent::builder()`;
- register file tools (`read_file`, `write_file`, `list_dir`) defined with the
  `#[everruns::tool]` attribute macro — no hand-written JSON Schema;
- open a `Session`, observe its `events()` (tool activity), and run prompts that
  accumulate history across turns.

## Run

Offline (deterministic simulator — no credentials, no network):

```bash
cargo run -p everruns-coding-cli -- --offline "list the files in src"
```

Against a real model (set `OPENAI_API_KEY`; optionally pick a model):

```bash
export OPENAI_API_KEY=sk-...
cargo run -p everruns-coding-cli -- --model gpt-5-mini "add a doc comment to lib.rs"
```

With no prompt argument it starts an interactive REPL (`Ctrl-D` to exit). Use
`-C/--cwd <dir>` to point the file tools at a workspace other than the current
directory; the tools reject absolute paths and `..` traversal so a tool call
cannot escape that root.

## Test

```bash
cargo test -p everruns-coding-cli
```

The tests cover the file tools (round-trip and traversal rejection), an offline
turn and a two-prompt session, and a source/manifest scan that fails if the
example ever reaches for `everruns-core`/`everruns-host`.

## Scope

This example favors a focused public-API loop over breadth. The previous
ratatui TUI, local MCP servers, and provider-specific wiring depended on host
internals the facade does not expose; they are intentionally omitted here.
