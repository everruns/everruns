# Research Agent

A self-contained Framework research agent. It uses OpenRouter's
`z-ai/glm-5.2` with the first-party `brave_web_search` capability, then cites source URLs and
separates facts from inferences.

## Run

![Research Agent terminal demo](demo.gif)

```bash
OPENROUTER_API_KEY=... BRAVE_SEARCH_API_KEY=... cargo run -p everruns-research-agent
cargo test -p everruns-research-agent
```

The test only validates agent construction; `cargo run` makes the real provider
and web-search calls.

The integration is configured with `BraveSearch::from_env()` and shares its
request and result handling with the hosted Platform capability. The example
contains no custom search HTTP client.

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
