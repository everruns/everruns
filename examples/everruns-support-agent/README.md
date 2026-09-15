# Everruns Support Agent

Answer Framework questions by searching and reading a small, inspectable corpus of official documentation. The agent retrieves evidence before answering instead of routing keywords to prewritten responses.

![Everruns Support Agent terminal demo](demo/demo.gif)

## What you learn

A two-step search/read tool interface, citable evidence, and an explicit boundary around the available knowledge.

## Scenario and expected outcome

The default question asks how to resume a durable session after restarting a process. The agent should search for persistence and session history, read the relevant pages, then explain the local catalog, persisted `SessionId`, and agent reattachment requirements with source URLs.

It must not imply that `Engine::new()` survives a restart. When the corpus lacks an answer, it should say so instead of inventing one.

## Run it

Install Rust/Cargo, clone the repository, and run from its root. This folder is self-contained **within the workspace**: its Cargo manifest references local Framework crates, so copying the folder alone is not sufficient.

```bash
git clone https://github.com/everruns/everruns.git
cd everruns
export ANTHROPIC_API_KEY="your-key"
cargo run -p everruns-framework-support-agent
```

The configured model is `claude-opus-5`. Provider access and funded credits are required. Keep keys in your environment, not in source control. Missing credentials, provider errors, and unsuccessful turns exit nonzero.

Try another question:

```bash
cargo run -p everruns-framework-support-agent -- "How do I register a custom provider?"
cargo run -p everruns-framework-support-agent -- "How can a tool return a structured error?"
```

Or type a question interactively:

```bash
cargo run -p everruns-framework-support-agent -- --interactive
```

## Build the agent

The definition lives in `src/agent.rs`; `main.rs` only handles input and runs the session. The prompt and documentation corpus live under `src/resources/`. Tools retrieve evidence; Opus decides what to search, read, and explain.

```rust
pub fn build(api_key: String) -> Result<Agent, BuildError> {
    Agent::builder()
        .name("everruns-support-agent")
        .instructions(include_str!("resources/instructions.md"))
        .provider(everruns_anthropic::provider("anthropic", api_key))
        .model(MODEL)
        .max_iterations(12)
        .tool(tools::search_docs())
        .tool(tools::read_doc())
        .build()
}
```

## Send, observe, and wait

The Framework interaction stays small in `main.rs`. The shared demo helper subscribes before sending, shows bounded tool previews, waits for completion, and rejects unsuccessful turns. Use `session.send_and_wait(&question).await?` when a live tool timeline is unnecessary.

```rust
let agent = agent::build(api_key)?;
let engine = Engine::new();
let session = engine.create(agent);

println!("MODEL: {}", agent::MODEL);
demo::run(&session, &question).await?;
```

This engine is in-memory. The agent can explain durable sessions from its corpus, but the example itself does not persist its session.

## How the tools work

`search_docs` ranks matches against five bundled pages and returns page IDs, public URLs, and matching excerpts. `read_doc` accepts only those page IDs and returns the complete snapshot. It never accepts an arbitrary filesystem path.

## Validate the behavior

```bash
cargo test -p everruns-framework-support-agent
bash examples/everruns-support-agent/demo/record.sh --check
```

Tests cover content-based search, empty and unmatched queries, complete citable reads, arbitrary-path rejection, and interactive input validation. They do not grade the model's answer; compare a live response with the expected outcome above.

CI runs the offline checks without provider credentials. Live model behavior is evaluated separately.

## Demo and recording

The screencast types a question into the same interactive binary shown above, then displays the actual Opus tool calls and answer. VHS hides most provider wait time but does not replace the model or tools with scripted output. Read `demo/transcript.txt` at your own pace.

With credentials exported and VHS, ffmpeg, and a VHS-compatible browser installed:

```bash
bash examples/everruns-support-agent/demo/record.sh
```

The script uses an exported `ANTHROPIC_API_KEY` when present, otherwise Doppler project `everruns-dev`, config `dev`. It updates `demo/demo.gif` and `demo/transcript.txt` only after a successful turn.

## Adapt it

Replace the bundled pages with a versioned documentation index while retaining separate search and read operations. Attach source/version metadata, restrict reads to authorized documents, and treat retrieved text as untrusted evidence rather than instructions.

## Boundaries

The corpus is a five-page snapshot from 2026-09-08, not a live search of docs.everruns.com. Provenance is recorded in `src/resources/docs/README.md`, and the snapshot can lag current APIs.

## Source map

`src/main.rs`: input and session execution; `src/agent.rs`: agent definition; `src/tools.rs`: bounded documentation retrieval; `src/resources/`: prompt and documentation corpus; `demo/`: live VHS recording, transcript, and recording script. `examples/demo-support` handles shared terminal presentation.
