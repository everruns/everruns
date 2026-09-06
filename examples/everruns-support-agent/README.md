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

Agent setup and tools live in [src/main.rs](src/main.rs). The separate
[src/demo.rs](src/demo.rs) subscribes to session events before sending the
question, prints real tool arguments and bounded result previews, and displays
the final answer. These tools expose public or demo data; review what may be
printed before adapting this observer to private data.

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
