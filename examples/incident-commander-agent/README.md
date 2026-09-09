# Incident Commander Agent

A self-contained Framework incident-coordination agent. It uses Meta Model
API's `muse-spark-1.3`, records a bounded incident update, and proposes safe
next actions from a real model response.

## Run

![Incident Commander Agent terminal demo](demo.gif)

```bash
MODEL_API_KEY=... cargo run -p everruns-incident-commander-agent
cargo test -p everruns-incident-commander-agent
```

Pass a different incident scenario after `--`. Tests validate agent construction
and log persistence without credentials; `cargo run` makes the real provider call.

`META_API_KEY` is also accepted for environments that use that name.

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

The tool appends each update to `incident.log` in this example folder. The log
survives program exits, is ignored by Git, and does not change production systems.
