---
title: Migrate between LLM providers
description: Switch agents from OpenAI to Anthropic to Gemini (or any OpenAI-compatible provider) without rewriting prompts or capabilities.
---

Everruns abstracts the LLM behind a uniform interface, so the same agent can run on OpenAI, Anthropic, Gemini, or any OpenAI-compatible provider. This guide swaps providers cleanly without losing sessions or rewriting agents.

## Concepts

Two pieces decide which model runs:

- **LLM Provider**: a configured API provider with encrypted credentials (e.g., `openai`, `openrouter`, `anthropic`, `gemini`, `openai_completions`).
- **LLM Model**: a specific model on a provider (e.g., `gpt-4o`, `claude-sonnet-4`, `gemini-1.5-pro`).

Model resolution priority on each turn:

1. Message-level `model` control (if present on the incoming message).
2. Session override (`session.default_model_id`).
3. Agent default (`agent.default_model_id`).
4. System default.

## Add a new provider

```bash
curl -X POST http://localhost:9300/api/v1/providers \
  -H "Content-Type: application/json" \
  -d '{
    "name": "anthropic",
    "provider_type": "anthropic",
    "api_key": "sk-ant-..."
  }'
```

The API key is encrypted at rest. The provider's models are discovered on creation; manually-added models are also supported.

## Switch an agent's default model

```bash
curl -X PATCH http://localhost:9300/api/v1/agents/$AGENT_ID \
  -H "Content-Type: application/json" \
  -d '{ "default_model_id": "model_claude_sonnet_4" }'
```

New sessions inherit the new default. **Existing sessions keep running on the model they started with** unless you override per-session or per-message.

## Override per session

For an A/B comparison, override at the session level:

```bash
curl -X POST http://localhost:9300/api/v1/sessions \
  -H "Content-Type: application/json" \
  -d '{
    "agent_id": "agent_...",
    "default_model_id": "model_claude_sonnet_4"
  }'
```

## Override per message

The most targeted form, run a single turn on a different model:

```python
await client.messages.create(
    session.id,
    "Re-analyse the above with extra rigour.",
    model="model_claude_opus",
)
```

## Compatibility caveats

Most behaviour ports across providers, but a few things differ:

- **Extended thinking / reasoning effort.** Supported on Anthropic Claude, OpenAI GPT-5.x, and o-series. Other models silently ignore the `reasoning_effort` control.
- **Execution phases on the wire.** OpenAI Responses API on the GPT-5.4 and GPT-5.5 families accepts `phase` annotations on replayed messages; other providers (and earlier OpenAI models) ignore them. Internal tracking continues regardless. The authoritative list is the `supports_phases` flag in `crates/core/src/llm_model_profiles.rs`. See [Execution phases](/explanation/agentic-loop/#execution-phases).
- **Tool call format differences.** The platform handles translation, but very large tool schemas may compress better on one provider than another.
- **Per-token cost.** USD budgets adjust automatically since they use per-model pricing. Token budgets are model-agnostic.

## Verify before flipping production

A safe migration sequence:

1. Add the new provider and verify discovered models.
2. Create a test agent that mirrors production, with `default_model_id` set to a model on the new provider.
3. Run an evaluation suite against the test agent.
4. Once happy, update the production agent's `default_model_id`, new sessions migrate over.
5. Leave old sessions on the old model; they age out naturally.

## See also

- [Concepts: LLM Provider and Model](/explanation/concepts/), entity model.
- [Observability with Braintrust](/observability/braintrust/), evaluate cross-provider quality.
