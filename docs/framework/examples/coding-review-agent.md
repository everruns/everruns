---
title: Coding Review Agent
description: Review a self-contained change with Claude Sonnet and a typed code-inspection tool.
---

The [Coding Review Agent source](https://github.com/everruns/everruns/tree/main/examples/coding-review-agent)
uses `claude-sonnet-5` to inspect and review the included
`sample_payment.rs`. The agent must read the change through its typed tool and
then report only material, reproducible findings.

![Coding Review Agent terminal demo](https://raw.githubusercontent.com/everruns/everruns/main/examples/coding-review-agent/demo.gif)

This screencast replays a real run in readable pages, with waiting time removed.
[Read the complete displayed transcript](https://github.com/everruns/everruns/blob/main/examples/coding-review-agent/demo.txt).
Tool-result previews are shortened; the agent receives the full tool response.

```bash
ANTHROPIC_API_KEY=... cargo run -p everruns-coding-review-agent
```

Pass a different review instruction after `--` to change the review focus.

## The important part

```rust
// The tool is what the reviewer is allowed to see. It serves one embedded
// file and rejects every other path, so the model cannot turn a review
// request into a file-read primitive over the host.
#[everruns::tool]
/// Read the self-contained code change that this example asks the agent to review.
async fn inspect_change(path: String) -> Result<String, String> {
    if path != "sample_payment.rs" {
        return Err("This self-contained example exposes only sample_payment.rs.".into());
    }
    Ok(include_str!("../sample_payment.rs").into())
}

let agent = Agent::builder()
    .name("coding-review-agent")
    // The instructions carry the review standard, not the code: what counts as
    // a finding, and what each finding must include.
    .instructions("You are a code reviewer. Use inspect_change before reviewing \
        the sample. Report only material, reproducible findings. For each finding, \
        explain impact, point to the relevant code, and name a focused validation.")
    // Anthropic is a driver crate; the agent builder takes any provider.
    .provider(everruns_anthropic::provider("anthropic", api_key))
    .model("claude-sonnet-5")
    .tool(inspect_change())
    .build()?;
```
