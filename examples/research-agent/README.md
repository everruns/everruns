# Research Agent

Search for relevant material, open primary sources, and synthesize a short, cited research brief. The model must read source content rather than treating search snippets as sufficient evidence.

## What you learn

Composing the first-party Brave Search and WebFetch capabilities, handling external evidence, and reporting uncertainty.

## Scenario and expected outcome

The default question asks what durable execution guarantees about retries and external side effects. A useful answer must distinguish replaying recorded results from the risk of retrying an unrecorded side effect.

Read at least two primary sources successfully before answering. Cite those pages, distinguish documented guarantees from inference, and explain why idempotency can still be necessary. If a source is inaccessible, try another and disclose the gap. Search snippets alone do not meet the task.

## Run it

Install Rust/Cargo, clone the repository, and run from its root. These folders are self-contained **within the workspace**: their Cargo manifests reference the local Framework crates, so copying one folder alone is not sufficient.

```bash
git clone https://github.com/everruns/everruns.git
cd everruns
export OPENROUTER_API_KEY="your-key"
export BRAVE_SEARCH_API_KEY="your-key"
cargo run -p everruns-research-agent
```

The configured model is `z-ai/glm-5.2`. Provider access and funded credits are required; a model identifier alone does not grant access. Keep keys in your environment, not in source control. Missing variables, provider errors, or unsuccessful turns exit nonzero.

Try a contrasting question:

```bash
cargo run -p everruns-research-agent -- "Compare retry guarantees in Temporal Activities and Restate. Read official sources and give two findings plus one caveat."
```

## Build the agent

This is the actual builder from `src/main.rs`. The prompt is an editable `instructions.md` file. Tools/capabilities supply evidence and actions; the model chooses how to use them.

```rust
let agent = Agent::builder()
    .name("research-agent")
    .instructions(include_str!("../instructions.md"))
    .provider(everruns_openrouter::provider("openrouter", api_key))
    .model(MODEL)
    .max_iterations(12)
    .capability(BraveSearch::from_env()?)
    .capability(everruns::WebFetch::new())
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

`BraveSearch::from_env()` provides `brave_web_search`. `WebFetch::new()` provides `web_fetch`, enabled through the Framework `web-fetch` Cargo feature. Search chooses candidates; fetch retrieves page content using the existing integration's egress controls. Download-to-file is not enabled.

## Validate the behavior

```bash
cargo test -p everruns-research-agent
python3 examples/research-agent/render_demo.py --check
```

Offline tests cover evidence-preview rendering and recording pagination; they do not perform web research. Validate a live run by checking successful `web_fetch` results for at least two primary sources and matching the final citations to pages actually read. Network/provider behavior and factual quality are not guaranteed by a green offline test.

CI runs these offline checks without provider credentials. Live model behavior is evaluated separately; passing tests is not proof of answer quality.

## Demo and recording

![Research Agent recorded run](demo.gif)

Read the [captured transcript](demo.txt) at your own pace. The GIF is a paginated replay of an actual provider run, with waiting time removed. It is not interactive and does not show model reasoning. Result excerpts are shortened only for display.

With credentials exported and Python 3, VHS, ffmpeg, and a VHS-compatible browser installed:

```bash
cd examples/research-agent
bash record.sh
```

The script captures a successful run, generates correctly wrapped pages and page durations, and renders `demo.gif`. It preserves the previous transcript when the provider run fails. To replay an existing transcript without another model call, run `python3 render_demo.py && vhs demo.tape`. `demo.txt` retains the displayed output; `.demo-pages/` is generated and ignored. Inspect results before sharing: public/demo data is safe here, but adapting tools may expose private data.

## Adapt it

Narrow the research question and source policy, add an evidence store if results need to survive sessions, and validate citation coverage before publishing important findings. Treat fetched text as untrusted data, not as instructions.

## Boundaries

Requires both OpenRouter and Brave Search credentials plus outbound HTTPS. Search and fetch can fail; twelve agent iterations cap the loop, not the bill. Word limits are instructions, not a hard output validator. This is a small research workflow, not an exhaustive literature review.

## Source map

`src/main.rs`: agent, search/fetch capabilities, and session; `instructions.md`: primary-source and evidence policy. `examples/demo-support` handles bounded source previews and shared terminal presentation; `record.sh` and `render_demo.py` handle recording.
