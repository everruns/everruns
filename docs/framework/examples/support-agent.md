---
title: Support Agent
description: Answer customer support questions with GPT-5.6 Terra and a typed customer-state tool.
---

The [Support Agent source](https://github.com/everruns/everruns/tree/main/examples/support-agent)
is a complete Framework program. It builds an `Agent`, connects OpenAI,
attaches a typed lookup tool, creates a session through `Engine`, and answers a
real support question.

![Support Agent terminal demo](https://raw.githubusercontent.com/everruns/everruns/main/examples/support-agent/demo.gif)

This screencast replays a real run in readable pages, with waiting time removed.
[Read the complete displayed transcript](https://github.com/everruns/everruns/blob/main/examples/support-agent/demo.txt).
Tool-result previews are shortened; the agent receives the full tool response.

```bash
OPENAI_API_KEY=... cargo run -p everruns-support-agent
```

The included `cust_demo` record is deliberately non-sensitive. Supply a
different question after `--` to try another support flow. CI builds the agent
with a placeholder credential; running the binary makes the real model call.

## The important part

The whole agent, minus the CLI plumbing:

```rust
// `#[everruns::tool]` derives the tool from the function itself: the doc
// comment becomes the description the model reads, and the typed parameters
// become its JSON schema. Returning `Err` hands the model a recoverable
// error instead of failing the turn.
#[everruns::tool]
/// Look up the safe, public support state for a demo customer.
async fn lookup_customer(customer_id: String) -> Result<String, String> {
    match customer_id.as_str() {
        "cust_demo" => Ok("cust_demo: verified account; password reset completed; \
            no active lockout; next safe action is to retry in a private browser window.".into()),
        // The tool is the trust boundary: it answers for one known record and
        // refuses everything else, so no model input can widen the lookup.
        _ => Err("Only the self-contained cust_demo record is available in this example.".into()),
    }
}

let agent = Agent::builder()
    .name("support-agent")
    .instructions("You are a customer-support agent. Use lookup_customer before \
        answering account questions. Do not expose private data.")
    .provider(OpenAI::new(api_key)) // provider and model are chosen separately
    .model("gpt-5.6-terra")
    .tool(lookup_customer()) // calling the macro-generated constructor attaches it
    .build()?;

// The Engine owns runtime resources and snapshots the agent into a session.
let session = Engine::new().create(agent);
// The example subscribes to session events so it can print each tool call as
// it happens; when you only want the answer, one call does it:
let turn = session.send_and_wait(question).await?;
println!("{}", turn.response);
```
