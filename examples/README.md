# Framework examples

Start here for complete, readable agent workflows. Each folder owns its prompt,
tools/capabilities, test data, README, and recording scripts. Clone the whole
repository: these Cargo packages depend on local workspace crates.

| Example | Learn | Run from repository root |
| --- | --- | --- |
| [Support](support-agent/) | Combine account facts and policy | `cargo run -p everruns-support-agent` |
| [Everruns Support](everruns-support-agent/) | Search and read a citable docs corpus | `cargo run -p everruns-framework-support-agent` |
| [Code Review](coding-review-agent/) | Execute a fixed reproduction before reporting a defect | `cargo run -p everruns-coding-review-agent` |
| [Research](research-agent/) | Search and fetch primary sources | `cargo run -p everruns-research-agent` |
| [Incident Commander](incident-commander-agent/) | Investigate evidence and persist a safe update | `cargo run -p everruns-incident-commander-agent` |
| [Bashkit Repo](bashkit-repo-agent/) | Modify and verify a repository through a sandboxed shell | `cargo run -p everruns-bashkit-repo-agent` |

Each README lists credentials, contrasting scenarios, expected outcomes, and
limits. Live runs incur provider/search charges. Offline tests exercise tool
behavior, not model quality. The default engine is in-memory, not durable storage.

`src/main.rs` shows build → create session → run → verify. `instructions.md` is
the editable prompt. Shared terminal presentation lives in `demo-support`; it
does not change agent behavior.

[Public walkthroughs](https://docs.everruns.com/framework/examples/) explain the code.
[Focused Framework API examples](../crates/everruns/examples/) cover persistence,
session history, workspaces, cancellation, and other individual features.
[Platform definitions](agents/) are a separate hosted-control-plane catalog.
