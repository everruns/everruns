# Support Agent

A self-contained Everruns Framework customer-support agent. It uses OpenAI's
`gpt-5.6-terra`, looks up the safe demo customer record through a typed tool,
and answers a real support question.

## Run

![Support Agent terminal demo](demo.gif)

```bash
OPENAI_API_KEY=... cargo run -p everruns-support-agent
cargo test -p everruns-support-agent
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
