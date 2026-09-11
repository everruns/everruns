---
title: Support Agent
description: Typed read-only tools, separating facts from policy, and choosing different answers for different inputs.
---

[Browse the complete example](https://github.com/everruns/everruns/tree/main/examples/support-agent).

Diagnose a sign-in problem by combining account facts with an explicit recovery policy. The interesting decision is whether the user needs MFA recovery, must wait for a lockout, or should try a clean browser session.

## What you learn

Typed read-only tools, separating facts from policy, and choosing different answers for different inputs.

## Scenario and expected outcome

The default customer reset their password, but has MFA enabled and neither an authenticator nor recovery codes. A password reset alone cannot solve this case.

For `cust_mfa`, direct the user to verified identity recovery, explicitly noting that resetting a password does not disable MFA. Never request passwords or recovery codes. For `cust_locked`, explain the 15-minute wait. For `cust_browser`, suggest a private window and an escalation if it still fails.

## Run it

Install Rust/Cargo, clone the repository, and run from its root. These folders are self-contained **within the workspace**: their Cargo manifests reference the local Framework crates, so copying one folder alone is not sufficient.

```bash
git clone https://github.com/everruns/everruns.git
cd everruns
export OPENAI_API_KEY="your-key"
cargo run -p everruns-support-agent
```

The configured model is `gpt-5.6-terra`. Provider access and funded credits are required; a model identifier alone does not grant access. Keep keys in your environment, not in source control. Missing variables, provider errors, or unsuccessful turns exit nonzero.

Try a contrasting question:

```bash
cargo run -p everruns-support-agent -- "cust_locked reset their password but cannot sign in. What should they do?"
cargo run -p everruns-support-agent -- "cust_browser cannot sign in after a reset. What next?"
```

## Build the agent

This is the actual builder from `src/main.rs`. The prompt is an editable `instructions.md` file. Tools/capabilities supply evidence and actions; the model chooses how to use them.

```rust
let agent = Agent::builder()
    .name("support-agent")
    .instructions(include_str!("../instructions.md"))
    .provider(OpenAI::new(api_key))
    .model(MODEL)
    .max_iterations(12)
    .tool(tools::lookup_customer())
    .tool(tools::read_support_policy())
    .build()?;
```

## Send, observe, and wait

The Framework interaction stays readable in `main.rs`. The shared demo helper subscribes before sending, filters events to this turn, shows bounded tool previews, waits for completion, and rejects unsuccessful turns. It changes presentation only; use `session.send_and_wait(question).await?` when you do not need the live tool timeline.

```rust
let engine = Engine::new();
let session = engine.create(agent);
println!("MODEL: {MODEL}");
demo::run(&session, question).await?;
```

This engine is in-memory. It does not demonstrate durable session storage; the Everruns Support example can explain that API, but does not itself persist its session.

## How the tools work

`lookup_customer` reads one of three fictional records from `customers.json`. It returns facts, not a prewritten recommendation. `read_support_policy` returns the recovery rules from `policy.md`. The model combines the two; no tool disables MFA or changes a real account.

## Validate the behavior

```bash
cargo test -p everruns-support-agent
python3 examples/support-agent/render_demo.py --check
```

Tests cover the distinct account states and rejection of unknown IDs. They do not grade the model's recommendation: compare a live response with the expected outcomes above.

CI runs these offline checks without provider credentials. Live model behavior is evaluated separately; passing tests is not proof of answer quality.

## Demo and recording

The earlier recording covered a simpler scenario and is not embedded here as evidence for this workflow. Use the command above to exercise this version, or record it with the script below.

With credentials exported and Python 3, VHS, ffmpeg, and a VHS-compatible browser installed:

```bash
cd examples/support-agent
bash record.sh
```

The script captures a successful run, generates correctly wrapped pages and page durations, and renders `demo.gif`. It preserves the previous transcript when the provider run fails. To replay an existing transcript without another model call, run `python3 render_demo.py && vhs demo.tape`. `demo.txt` retains the displayed output; `.demo-pages/` is generated and ignored. Inspect results before sharing: public/demo data is safe here, but adapting tools may expose private data.

## Adapt it

Replace the fixture lookup with your authorized customer-data service. Keep policy separate from account facts, scope lookups to the authenticated customer, and return only fields needed for the support decision. Add human approval before any account mutation.

## Boundaries

All customers, policy rules, and support.example.com URLs are fictional. This is a read-only support exercise, not a live help desk.

## Source map

`src/main.rs`: agent and session; `src/tools.rs`: bounded account lookup and policy tool; `customers.json`: three contrasting cases; `policy.md`: recovery rules; `instructions.md`: agent instructions. `examples/demo-support` handles shared terminal presentation; `record.sh` and `render_demo.py` handle recording.
