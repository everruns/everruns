---
title: Code Mode with MCP
description: Let an agent call a large MCP server's tools from Bashkit scripts, so their schemas stay out of the prompt and many calls take one step.
---

Attach an MCP server with hundreds or thousands of tools natively and every
model request carries every tool schema. Code mode moves those tools into the
sandboxed [Bashkit shell](/capabilities/bashkit-shell/) instead. The model sees
one `bash` tool, finds operations with `tools search`, and writes scripts that
call them, join the results with `jq`, and print only the answer. The
intermediate payloads never enter the conversation.

![Code mode over a 1,000-tool MCP server, terminal demo](https://raw.githubusercontent.com/everruns/everruns/main/examples/code-mode-mcp-agent/demo/demo.gif)

The demo is the [Code Mode MCP Agent](https://github.com/everruns/everruns/tree/main/examples/code-mode-mcp-agent)
example running `gpt-6.1-sol` against a synthetic MCP server with 1,000 tools
(355 KiB of schemas). Each `❯ bash` block is a script the model wrote; each
`↳ tools acme ...` line under it is an MCP tool that script called, with its
input. The model's tool list stayed at four tools and about 4 KiB, its largest
prompt was about 3,500 tokens, and the answer matched totals computed from the
server's data.

## Declare it

Code mode is three lines on the agent builder: the shell, the
[Shell Code Mode](/capabilities/shell-code-mode/) capability in `cli` mode, and
the MCP server. Enable the `bashkit` and `mcp-stdio` (or `mcp`) features.

```toml
[dependencies]
everruns = { version = "*", features = ["bashkit", "mcp-stdio", "openai"] }
serde_json = "1"
```

```rust
use everruns::{Agent, BashkitShell, CapabilityRef, Engine, McpServer, OpenAI};
use serde_json::json;

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let agent = Agent::builder()
    .name("acme-analyst")
    .instructions(
        "The company APIs are available only in the shell, through the `tools` \
         command: find operations with `tools search`, read one with `--help`, \
         then do the work in one script and print only what you need.",
    )
    .provider(OpenAI::from_env()?)
    .model("gpt-6.2-sol")
    .capability(BashkitShell::new())
    .capability(
        CapabilityRef::new("shell_code_mode")
            .config(json!({"mode": "cli", "rate_unlabeled_tools": false})),
    )
    .mcp_server(McpServer::stdio("acme", "acme-mcp-server"))
    .build()?;

let session = Engine::new().create(agent);
let turn = session
    .run("Which EU customers have overdue invoices? Give the top three by amount.")
    .await?;
println!("{}", turn.response);
# Ok(())
# }
```

`mode: "cli"` removes every eligible tool from the model's tool list and makes
it a `tools <server> <operation>` command in the shell. `mode: "hybrid"` keeps
the native route as well. `rate_unlabeled_tools: false` skips the decision
service for tools that declare no read-only or destructive hint; leave it on
when your server does not label its tools. Approvals still apply: a call that
needs approval stops the script and asks. See
[Shell Code Mode](/capabilities/shell-code-mode/) for every option, and
[Large MCP catalogs](/features/large-mcp-catalogs/) for the limits on catalog
size and discovery.

## What the model writes

In a run of the example, after `tools search invoices customers`, the two
operations' `--help` and one script that sampled the data, the model wrote this
script. It is copied unedited from that run; the two MCP calls below it ran
inside the shell:

```bash
set -euo pipefail
fetch_all() {
 local op="$1" base="$2" cursor='' page args all='[]'
 while :; do
  args=$(jq -cn --argjson base "$base" --arg cursor "$cursor" '$base + {limit:500} + (if $cursor == "" then {} else {cursor:$cursor} end)')
  page=$(tools acme "$op" "$args")
  all=$(jq -cn --argjson all "$all" --argjson page "$page" '$all + $page.items')
  cursor=$(jq -r '.next_cursor // empty' <<< "$page")
  [[ -n "$cursor" ]] || break
 done
 printf '%s' "$all"
}
customers=$(fetch_all crm-list-customers '{"region":"EU"}')
invoices=$(fetch_all billing-list-invoices '{"status":"overdue"}')
jq -n --argjson customers "$customers" --argjson invoices "$invoices" '[ $customers[] as $c | [$invoices[] | select(.customer_id == $c.id)] as $overdue | select(($overdue|length)>0) | {customer:$c.name,total_overdue_usd:($overdue|map(.amount_usd)|add)} ] | sort_by(-.total_overdue_usd) | .[:3]'
```

```text
↳ tools acme crm-list-customers {"limit":500,"region":"EU"}
↳ tools acme billing-list-invoices {"limit":500,"status":"overdue"}
```

A `tools` call prints the tool's JSON reply as is, so a script pipes it straight
into `jq`. When an MCP server answers in text, the text is printed unchanged.

## Watch the calls

Each call a script makes is a `tool.nested_call` event on the session's event
stream, with the tool name, the command as the script spelled it, a shortened
input, and a status. The model's own `bash` calls stay ordinary `tool.started`
and `tool.completed` events, so a host can tell the two apart:

```rust
# async fn watch(session: everruns::Session) -> Result<(), Box<dyn std::error::Error>> {
let mut events = session.events();
// ... run the turn ...
while let Some(event) = events.try_recv()? {
    if event.event_type() == "tool.nested_call" {
        let data = &event.canonical_json()["data"];
        println!("{} {}", data["command"], data["input_preview"]);
    }
}
# Ok(())
# }
```

## Run the example

From a checkout of the repository:

```bash
export OPENAI_API_KEY="your-key"       # or OPENROUTER_API_KEY
cargo run -p everruns-code-mode-mcp-agent
```

Add `-- --interactive` to type your own question. The MCP server is the same
binary, started again with `--mcp-server`, so nothing outside the repository is
needed. After the turn the example checks that no MCP tool reached the model's
tool list, that the model called none directly, that the calls ran inside
scripts, and that the answer is right. A failed check exits nonzero.

## Other execution interfaces

This page describes shell code mode. [Python Code Mode](/framework/python-code-mode/)
uses Monty for direct Python orchestration and saved approval continuations.
Bashkit already includes Python; compare the execution contracts in
[choosing an execution option](/framework/execution-options/).
