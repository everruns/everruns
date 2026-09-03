---
title: OpenTelemetry
description: Export Everruns agent traces over OTLP. Spans follow the OpenTelemetry Gen-AI and OpenInference conventions, so any tracing backend reads them.
sidebar:
  order: 1
---

Everruns turns every agent run into an OpenTelemetry trace and exports it over OTLP. Spans carry two attribute vocabularies at once, the [OpenTelemetry Gen-AI semantic conventions](https://github.com/open-telemetry/semantic-conventions-genai/tree/main/docs/gen-ai) and the [OpenInference conventions](https://arize-ai.github.io/openinference/spec/semantic_conventions.html), so one endpoint feeds general-purpose backends such as Grafana Tempo, Jaeger, and Datadog as well as LLM-native ones such as Arize Phoenix and Langfuse.

## What You Get

- **A trace per turn**: an `invoke_agent` root span named after your agent, with model calls, tool runs, and reasoning phases nested underneath
- **Real timings**: spans start and end at the moment each event happened, so waterfalls show true model latency and tool duration
- **Token and cost detail**: input, output, and prompt-cache tokens per call and per turn, plus cost where the provider reports it
- **Tool visibility**: every tool call is its own span with name, description, call id, and outcome
- **Failure detail**: a low-cardinality `error.type`, an error span status carrying the message, and an `exception` event
- **Privacy by default**: prompts, completions, reasoning, and tool payloads are never exported unless you turn them on

## Quick Start

### 1. Point Everruns at a collector

Set one variable. Any OTLP/HTTP endpoint works.

```bash
# Local collector, Grafana Tempo, Datadog agent, ...
export OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318

# Name the service in your traces
export OTEL_SERVICE_NAME=everruns-server
```

Traces are exported over OTLP HTTP/protobuf. Point the variable at the base endpoint, usually port `4318`, and Everruns appends the `/v1/traces` path; a full signal URL is used as given.

### 2. Configure

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Yes | - | OTLP/HTTP endpoint. Tracing stays off while unset |
| `OTEL_SERVICE_NAME` | No | `everruns-server`, `everruns-worker` | Service name on the spans |
| `OTEL_SERVICE_VERSION` | No | - | Service version on the spans |
| `OTEL_ENVIRONMENT` | No | - | Deployment environment label |
| `OTEL_SDK_DISABLED` | No | `false` | Disable tracing without unsetting the endpoint |
| `OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT` | No | `false` | Export instructions, messages, tool arguments and results |
| `EVERRUNS_TRACE_CONVENTIONS` | No | `gen_ai,openinference` | Which attribute vocabularies to write |

### 3. View traces

Open your backend and look for the `invoke_agent` spans. In Arize Phoenix, point the same variable at Phoenix and its spans appear as AGENT, LLM, and TOOL rows with no extra configuration:

```bash
export OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:6006
```

## Trace Structure

Each turn is one trace. Reasoning and acting phases give the trace its shape, and extended thinking sits inside the model call it belongs to:

```
invoke_agent {agent name}          (root, INTERNAL)
├── reason                         reasoning phase
│   └── chat {model}               model call (CLIENT)
│       └── thinking               extended thinking, when enabled
├── act                            tool execution phase
│   ├── execute_tool {name}
│   └── execute_tool {name}
├── reason
│   └── chat {model}
└── (no further tool calls, turn complete)
```

| Span | Kind | `gen_ai.operation.name` | `openinference.span.kind` |
|------|------|-------------------------|---------------------------|
| `invoke_agent {agent name}` | `INTERNAL` | `invoke_agent` | `AGENT` |
| `chat {model}` | `CLIENT` | `chat` | `LLM` |
| `execute_tool {name}` | `INTERNAL` | `execute_tool` | `TOOL` |
| `reason`, `act`, `thinking` | `INTERNAL` | none | `CHAIN` |

The reason, act, and thinking spans are Everruns phases rather than Gen-AI operations, so they carry no `gen_ai.operation.name` and backends do not count them as model calls.

## OpenTelemetry Gen-AI Support

| | Supported | Attributes |
|---|-----------|------------|
| ✅ | Agent span per turn | `gen_ai.agent.id`, `gen_ai.agent.name`, `gen_ai.agent.description` |
| ✅ | Model call spans | `chat {model}`, CLIENT kind, real call duration |
| ✅ | Provider and model identity | `gen_ai.provider.name`, `gen_ai.request.model`, `gen_ai.response.model`, `gen_ai.response.id` |
| ✅ | Finish reasons | `gen_ai.response.finish_reasons` as a string array |
| ✅ | Token usage with cache split | `gen_ai.usage.input_tokens`, `output_tokens`, `cache_read.input_tokens`, `cache_write.input_tokens` |
| ✅ | Request parameters | `gen_ai.request.temperature`, `max_tokens`, `reasoning.level`, `stream` |
| ✅ | Streaming latency and compaction | `gen_ai.response.time_to_first_chunk`, `gen_ai.conversation.compacted` |
| ✅ | Tool execution spans | `gen_ai.tool.name`, `gen_ai.tool.type`, `gen_ai.tool.call.id`, `gen_ai.tool.description` |
| ✅ | Extended thinking | Nested inside the model call it belongs to |
| ✅ | Errors | `error.type`, error span status, `exception` events |
| ✅ | Conversation correlation | `gen_ai.conversation.id` on every span |
| ✅ | Content capture (opt-in) | `gen_ai.system_instructions`, `gen_ai.input.messages`, `gen_ai.output.messages`, `gen_ai.tool.definitions`, `gen_ai.tool.call.arguments`, `gen_ai.tool.call.result` |
| ✅ | Accurate timestamps | Spans start and end at the times the events record |

Not emitted yet: `server.address` and `server.port` on model calls, and parameter schemas inside `gen_ai.tool.definitions`.

## OpenInference Support

| | Supported | Attributes |
|---|-----------|------------|
| ✅ | Span kinds | `openinference.span.kind`: `AGENT`, `CHAIN`, `LLM`, `TOOL` |
| ✅ | Session and agent identity | `session.id`, `agent.name`, `metadata` |
| ✅ | Model identity | `llm.model_name`, `llm.provider`, `llm.system` |
| ✅ | Token counts | `llm.token_count.prompt`, `.completion`, `.total` |
| ✅ | Prompt cache detail | `llm.token_count.prompt_details.cache_read`, `.cache_write` |
| ✅ | Cost | `llm.cost.total` in USD, when the provider reports it |
| ✅ | Invocation parameters and tools | `llm.invocation_parameters`, `llm.tools.N.tool.json_schema` |
| ✅ | Input and output values (opt-in) | `input.value`, `output.value`, with `input.mime_type` and `output.mime_type` |
| ✅ | Flattened messages (opt-in) | `llm.input_messages.N.message.*`, `llm.output_messages.N.message.*`, tool calls included |
| ✅ | Tool spans | `tool.name`, `tool.description`, arguments and results as input and output values |
| ✅ | Errors | Error span status with `exception` events |
| ✅ | Phoenix out of the box | Point the OTLP endpoint at Phoenix, nothing else to configure |

## Everruns Attributes

Spans also carry a few Everruns-specific attributes under their own namespace, so they never collide with either convention:

| Attribute | Spans | Description |
|-----------|-------|-------------|
| `everruns.turn.id`, `everruns.exec.id`, `everruns.input_message.id` | All | Correlation ids that match the [event stream](/features/events/) |
| `everruns.phase` | `reason`, `act`, `thinking` | Which phase the span represents |
| `everruns.turn.iterations`, `everruns.turn.tool_call_count`, `everruns.turn.llm_call_count`, `everruns.turn.status` | `invoke_agent` | Turn counters and outcome |
| `everruns.tool.status`, `everruns.tool.capability.id` | `execute_tool` | Tool outcome and the capability that provided it |
| `everruns.usage.cost_usd` | `chat`, `invoke_agent` | Cost when known |
| `everruns.llm.retry.attempts`, `everruns.llm.retry.total_wait_ms` | `chat` | Provider retries behind a single call |
| `everruns.span.orphaned`, `everruns.span.unterminated` | Any | Diagnostics: a span rebuilt from a terminal event, or closed because its turn ended first |

## Choosing Conventions

Both vocabularies are written by default. Narrow to one when a backend only reads one and you want smaller spans:

```bash
# OpenTelemetry Gen-AI only (Tempo, Jaeger, Datadog, Langfuse)
export EVERRUNS_TRACE_CONVENTIONS=gen_ai

# OpenInference only (Arize Phoenix)
export EVERRUNS_TRACE_CONVENTIONS=openinference
```

Span names, kinds, and hierarchy are the same either way; only the attributes differ. An unrecognized value falls back to writing both.

## Content Capture

Prompts, completions, reasoning, and tool payloads are **not** exported by default. Turn them on with the standard OpenTelemetry variable:

```bash
export OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT=true
```

With capture on:

- model call spans carry `gen_ai.system_instructions`, `gen_ai.input.messages`, `gen_ai.output.messages`, and `gen_ai.tool.definitions` in the conventions' `{role, parts}` JSON, plus the OpenInference `input.value` and `output.value` and the flattened `llm.input_messages.N.message.*` keys
- tool spans carry their arguments and results
- the turn root carries the user's message and the final answer
- the thinking span carries the model's reasoning text

Two details worth knowing. Everruns keeps the agent's instructions separate from the conversation, so system messages are exported as `gen_ai.system_instructions` and left out of `gen_ai.input.messages`. Image bytes are never copied into a span; an image becomes a reference that names its media type only.

Treat this as a data-retention decision. Everything captured leaves your deployment for whatever backend the OTLP endpoint points at.

## Troubleshooting

### No traces appear

1. Confirm `OTEL_EXPORTER_OTLP_ENDPOINT` is set and reachable from the server and worker processes; without it, tracing is off and startup logs say so
2. Confirm the endpoint speaks OTLP over **HTTP**, typically port `4318`, not the gRPC port `4317`
3. Check startup logs for `OpenTelemetry tracing enabled` with your endpoint, or a warning that the exporter failed to initialize
4. Confirm `OTEL_SDK_DISABLED` is not set to `true`

### Traces appear but spans look empty

1. Prompts and completions require `OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT=true`; without it, spans carry structure and metrics only, which is the default
2. If your backend reads only one vocabulary, check `EVERRUNS_TRACE_CONVENTIONS` has not been narrowed to the other one

### Turns are split across traces

Each turn is deliberately its own trace. Group by `gen_ai.conversation.id`, or `session.id` in Phoenix, to follow a whole session.

## Related

- [Braintrust](/observability/braintrust/), LLM observability and evaluation with a dedicated exporter
- [Events](/features/events/), the event stream every exporter is built on
- [Environment Variables](/sre/environment-variables/), the full operator reference
