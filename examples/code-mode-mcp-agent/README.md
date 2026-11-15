# Code Mode MCP Agent

Answer a question over an MCP server with 1,000 tools without putting a single
one of their schemas in the prompt. The agent writes [Bashkit](https://bashkit.sh)
scripts that find and call the server's tools through the `tools` command, joins
the results with `jq`, and prints only the answer. This is a real `gpt-6.1-sol`
agent, not a scripted turn.

![Code Mode MCP Agent terminal demo](demo/demo.gif)

## What you learn

How to declare code mode in the Framework: attach an MCP server, turn on
`bashkit_shell`, and add `tools_in_shell` in `cli` mode. The MCP tools leave the
model's tool list and become shell commands, so a huge catalog costs the prompt
nothing and a task that needs many calls takes a few scripts.

```rust
Agent::builder()
    .capability(BashkitShell::new())
    .capability(
        CapabilityRef::new("tools_in_shell")
            .config(json!({"mode": "cli", "rate_unlabeled_tools": false})),
    )
    .mcp_server(McpServer::stdio("acme", server_command).arg("--mcp-server"))
    // provider, model, instructions...
    .build()
```

The full declaration is in [`src/agent.rs`](src/agent.rs).

## Scenario and expected outcome

`acme` is a synthetic company API: 25 services (CRM, billing, inventory, HR,
shipping and 20 more), 8 entities each, 5 verbs per entity. That is 1,000 tools
and 355 KiB of tool schemas. The server is this same binary, re-executed with
`--mcp-server`; it pages `tools/list`, labels every tool read-only or
destructive, refuses every write, and holds deterministic data behind the
billing and CRM tools.

The question needs both services: which EU customers have overdue invoices,
and which three owe the most. The agent searches the catalog, reads the two
operations' help, and runs a script that lists overdue invoices and EU
customers, joins them, and sums per customer.

The demo prints each script the model wrote, every MCP tool those scripts
called (`↳ tools acme ...`), and what came back. After the turn, Rust code checks
that no MCP tool was in the model's tool list or called directly, that the MCP
calls ran inside scripts, and that the answer matches totals computed from the
server's data. Any failed check exits nonzero.

## Run it

Clone the repository and run from its root:

```bash
export OPENAI_API_KEY="your-key"       # or OPENROUTER_API_KEY
cargo run -p everruns-code-mode-mcp-agent
```

Type your own question:

```bash
cargo run -p everruns-code-mode-mcp-agent -- --interactive
```

The model is `gpt-6.1-sol`: through OpenRouter (`openai/gpt-6.1-sol`) when
`OPENROUTER_API_KEY` is set, straight from OpenAI otherwise. The answer check
runs only for the default question.

## Record the demo

[`demo/record.sh`](demo/record.sh) drives [VHS](https://github.com/charmbracelet/vhs)
through [`demo/demo.tape`](demo/demo.tape) against a live model and writes
`demo/demo.gif` plus a plain-text [`demo/transcript.txt`](demo/transcript.txt).
`demo/record.sh --check` validates the tape offline.
