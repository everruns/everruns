---
title: Anthropic
description: Run Everruns agents on Anthropic Claude models via the Messages API, with streaming, tool use, and extended thinking.
sidebar:
  label: Anthropic
---

<svg role="img" aria-label="Anthropic logo" width="56" height="56" viewBox="0 0 24 24" fill="currentColor" xmlns="http://www.w3.org/2000/svg" style="float: right; margin-left: 16px;"><path d="M17.304 3h-3.437l5.73 18h3.437L17.304 3zM6.696 3l-5.73 18H4.43l1.307-4.26h5.905L12.95 21h3.466L10.696 3H6.696zm.64 10.74L9.2 7.895l1.864 5.845H7.336z"/></svg>

Everruns runs agents on [Anthropic](https://www.anthropic.com/) Claude models
through the Claude Messages API, mapping its provider-neutral messages, tools,
and reasoning onto the Anthropic wire format.

## What you get

- **Claude Messages API** streaming.
- **Tool use** mapped to provider-neutral Everruns tools.
- **Extended thinking**: adaptive thinking on recent Claude families and
  budget-based thinking on older ones, with the chain-of-thought signature
  preserved across multi-turn conversations.
- **Prompt caching** via bounded `cache_control` breakpoints on stable, high-value
  sections of the request.

## Configure in Everruns

1. Go to **Settings** → **Providers** and click **Add provider**.
2. Choose **Anthropic**.
3. Paste your Anthropic API key. Get one from the
   [Anthropic Console](https://console.anthropic.com/).
4. Save. Everruns discovers available Claude models automatically.

### Claude Max and Team API credits

Claude Max and Team plans include monthly Claude API credits. They land in the
Anthropic Console organization you link to your plan, so any API key from that
organization spends them first, before purchased credits. To run Everruns
agents on those credits, create the key in the linked organization and add it
as described above. Nothing else in Everruns changes.

Everruns does not offer signing in with a Claude.ai account: Anthropic does not
allow third-party products to use Claude.ai login or plan usage limits. See
[API credits for Max and Team plans](https://platform.claude.com/docs/en/about-claude/api-credits-for-subscribers)
for eligibility, amounts, and how to link the organization.

## Message threads (beta)

Add the **Claude Message Threads** capability (`claude_message_threads`) to an
agent or harness to opt in. It uses Anthropic's
[Messages API threads](https://platform.claude.com/docs/en/build-with-claude/message-threads)
to send only the new messages after a completed response. Existing agents keep
sending full history unless this capability is enabled.

```json
{
  "ref": "claude_message_threads"
}
```

In the Framework (with the `anthropic` feature):

```rust
use everruns::{Agent, providers::anthropic::Anthropic};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _agent = Agent::builder()
        .provider(Anthropic::from_env()?)
        .model("claude-sonnet-5-5")
        .capability("claude_message_threads")
        .build()?;
    Ok(())
}
```

Everruns remains the source of truth for conversation history. A thread expires
about five minutes after its last request. Missing or already-used thread state
causes one replay of the full transcript in a new thread. A thread-specific
unsupported-request error falls back to a normal Messages request. Other errors
retain their normal handling.

Continuation state is held only in a bounded process-local cache. Worker changes,
restarts, cache eviction, edits, changed settings or credentials, and concurrent
branches can start fresh threads. A partial or cancelled response never advances
thread state. Tool results are supported; native compaction and opaque compacted
prefixes use the existing full-history path. Batch requests stay unthreaded.
Changes to a system prompt on every turn can prevent reuse.

This is separate from [prompt caching](/features/prompt-caching/): threads reduce
uploaded request history, but do not change token pricing or rate limits. Keep
prompt caching enabled independently when desired.

**Data retention:** this beta is not eligible for zero data retention or HIPAA.
Anthropic temporarily holds the conversation, system prompt, and tools, and
states that it deletes thread state within one hour of last use. Enable the
capability only when this retention is acceptable for your workload.

Low-level driver callers can set
`config.driver_options["anthropic/message_threads"] = serde_json::json!(true)`.
Supply a stable, unique `config.metadata["session_id"]` and the complete transcript
on every call, including assistant content and tool calls. Calls without a session
scope stay unthreaded. Other provider drivers ignore this Anthropic option.

## Models

Anthropic's `/v1/models` endpoint returns capability metadata, which Everruns
merges with its built-in model profiles. Hardcoded profiles take precedence for
cost data; discovered data fills gaps for newer models.

`max_tokens` is required on every Anthropic request, so Everruns always resolves
a value from the model profile (falling back to a safe default) and will retry
once with a lower limit if a stale profile causes the provider to reject it.

Thinking counts toward `max_tokens`. When you set `max_tokens` yourself, Everruns
preserves it as a hard limit on all generated tokens, including thinking. When
the limit cannot accommodate thinking, the driver omits thinking rather than
exceed the limit. On models that always think (Opus 5.5, Sonnet 5.5,
Haiku 5.5, Fable 5.x), omitting it would run thinking at the provider's default
effort, so the driver keeps it and lowers the effort to `low` instead. A very
small limit on those models can still be spent on thinking; give them room for
the answer.

Claude models that always think (Opus 5.5, Sonnet 5.5, Haiku 5.5, Fable 5.x) always get an explicit
effort: the model's default when you choose none, and `low` when you choose
`none`. These and the other adaptive-thinking models reject assistant prefill, so
a conversation must end with a user or tool message.

## Links

- [Anthropic](https://www.anthropic.com/)
- [Anthropic Console](https://console.anthropic.com/)
- [API credits for Max and Team plans](https://platform.claude.com/docs/en/about-claude/api-credits-for-subscribers)
- [`everruns-drivers` on crates.io](https://crates.io/crates/everruns-drivers), feature `anthropic`
- [Migrate between providers](/how-to/migrate-providers/)
