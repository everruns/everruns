# Runnable and importable agent examples

Every Framework example is a self-contained Cargo package with its own source,
test, README, typed tool, and model profile. Each executes a deterministic
scripted tool loop in CI; a production embedding replaces the simulated model
with its configured provider and model.

| Example | Run | Production model profile |
| --- | --- | --- |
| [Support](../support-agent) | `cargo run -p everruns-support-agent` | `gpt-5.6-terra` |
| [Everruns Support](../everruns-support-agent) | `cargo run -p everruns-framework-support-agent` | `claude-opus-5` |
| [Coding Review](../coding-review-agent) | `cargo run -p everruns-coding-review-agent` | `claude-sonnet-5` |
| [Research](../research-agent) | `cargo run -p everruns-research-agent` | `z-ai/glm-5.2` |
| [Incident Commander](../incident-commander-agent) | `cargo run -p everruns-incident-commander-agent` | `muse-spark-1.3` |

## Platform definitions

These are complete, importable Platform agent definitions. They pair a bounded
system prompt with the capabilities needed for a real workflow; they are not
prompt fragments. Import one into a running Everruns Platform instance:

```bash
everruns agents create --file examples/agents/customer-support-agent.md
```

The agent file deliberately does not pin `default_model_id`: Everruns stores
that as an organization-local model UUID, so copying a UUID from this repository
would make an example fail in every other organization. After import, choose
the listed provider and model in the agent settings (or resolve its UUID from
your organization's model list before using the CLI).

| Agent | Primary model profile | Why | Required provider |
| --- | --- | --- | --- |
| [Support Agent](customer-support-agent.md) | `gpt-5.6-terra` | Reliable interactive support and tool use | OpenAI |
| [Everruns Support Agent](everruns-support-agent.md) | `claude-opus-5` | Deep troubleshooting across docs and evidence | Anthropic |
| [Coding Review Agent](coding-review-agent.md) | `claude-sonnet-5` | Strong repository reasoning with a practical review loop | Anthropic |
| [Research Agent](research-agent.md) | `z-ai/glm-5.2` | Long-context, multi-step research via OpenRouter | OpenRouter |
| [Incident Commander Agent](incident-commander-agent.md) | `muse-spark-1.3` | Long-horizon incident coordination with a large context window | Meta Model API |

Connect the required provider before selecting its model. The OpenRouter profile
uses the provider model slug; synchronize that provider's catalog first. Model
availability is account-specific, so select the newest compatible profile if a
listed model is unavailable.

## Run a conversation

Create a session for the imported agent in the UI, or use the CLI with its
agent ID and an organization-local model ID:

```bash
everruns sessions create --agent AGENT_ID --model MODEL_ID \
  "Review the current pull request and report only material findings."
```

For local development, start the canonical stack from the repository root,
import the definition, then create a session. These definitions are checked in
the running Platform when imported; no provider credentials are needed until a
live session starts.
