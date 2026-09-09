# Everruns Support Agent

A self-contained Framework agent for developer support. It uses Anthropic's
`claude-opus-5`, consults authoritative Everruns documentation links, and
answers a real troubleshooting question.

## Run

![Everruns Support Agent terminal demo](demo.gif)

```bash
ANTHROPIC_API_KEY=... cargo run -p everruns-framework-support-agent
cargo test -p everruns-framework-support-agent
```

Pass a different support question after `--`. The test only validates agent
construction; `cargo run` makes the real provider call.

## How the demo works

This example is only its agent: [src/main.rs](src/main.rs) holds the tools,
instructions, provider, and model. The terminal observer is shared by every
example and lives in [examples/demo-support](../demo-support); it subscribes to
session events before sending the question, prints real tool arguments and
bounded result previews, and displays the final answer. Nothing there changes
what the agent does — `session.send_and_wait(question)` is the same run without
the live view. These tools expose public or demo data; review what may be
printed before pointing that observer at private data.

The screencast is a **paged replay of a recorded live run**, with provider wait
time removed. Read the [complete displayed transcript](demo.txt) at your own pace;
tool results marked `[preview]` are shortened only for display.

To capture a new live run, export the keys above and run:

```bash
bash record.sh
```

Recording needs VHS and `less`. Adjust the page count in `demo.tape` if a new
answer is longer; `vhs demo.tape` replays the saved transcript without API calls.

`search_docs` fetches Markdown content from the official Everruns repository,
using a fixed set of documentation pages and returning their public citation URLs.
