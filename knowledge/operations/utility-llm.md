---
type: Specification
title: "Utility LLM Service"
description: "Internal utility LLM service for capability internals."
tags:
  - everruns
  - operations
---
# Utility LLM Service

## Intent

Provide a system-owned LLM service for built-in capability internals.

The utility model is not an agent model provider, public API, UI option, or
session/agent configuration surface. It is a host service exposed through
capability execution context so capabilities can perform bounded internal model
work without reusing user-configured model providers or session secrets.

Server-side system analysis tasks are also sanctioned callers: bounded,
system-initiated analysis of platform-owned data (e.g., agent configuration
checks per `knowledge/evaluation/agent-checks.md`). These run inside server domains via the
`HostComposition` service handle, never as a public ad hoc completion
endpoint.

## Core Contract

`everruns-core` owns the service abstraction:

- `UtilityLlmService` is the async trait used by capability internals.
- `UtilityLlmRequest` is the provider-neutral request shape.
- `UtilityLlmReasoningEffort` allows `low`, `medium`, and `high` when a caller
  explicitly needs reasoning.
- `UtilityLlmService::is_configured()` reports whether the deployment has the
  service enabled.
- `HostComposition` carries the active service as part of the platform
  profile.
- Runtime tool execution threads the service into `ToolContext`.
- `everruns-host` owns the concrete implementation behind its optional
  `utility-llm` feature (`utility-openai` remains as an alias of that feature);
  the utility LLM service remains the capability-facing typed API.
- Utility LLM provider transport is host-owned. It does not route through
  `EgressService` and is not governed by tenant/agent egress policy such as
  `EVERRUNS_SYSTEM_ALLOWLIST_ENABLED`.

Backend and model are deployment configuration, not request data. A
`UtilityLlmRequest` carries no model, no tools, no tool search, no previous
response id, and no provider credentials; the host service supplies the model
when it converts the request for its driver. By default the service sends no
reasoning parameter; callers can opt into `low`, `medium`, or `high`.

## System Configuration

The service is configured from process environment:

- `UTILITY_OPENROUTER_API_KEY` — serve utility calls through OpenRouter
- `UTILITY_OPENAI_API_KEY` — serve them directly from OpenAI
- `UTILITY_LLM_MODEL` — override the model for the selected backend

Key presence selects the backend rather than a separate provider variable: an
operator who moves the deployment to OpenRouter sets one secret instead of
keeping a name and a key in sync. OpenRouter wins when both keys are present,
with a warning at startup, because the newly added key is the deliberate one.
Defaults are `gpt-5.6-luna` on OpenAI and `openai/gpt-5.6-luna` on OpenRouter —
the same model, named the way each backend names it.

When no key is set, the service is disabled. Disabled deployments should call
`is_configured()` before attempting optional utility work, or handle the
configuration error returned by completion methods.

The default server and worker platform profiles enable `everruns-host`'s
`utility-llm` feature and resolve `SystemUtilityLlmConfig::from_env()` during
platform construction. Embedders can bypass env-based setup by constructing a
custom `HostComposition` and calling
`HostComposition::builder().utility_llm_service(...)` without enabling the
concrete implementation.

## Non-Goals

- No user-facing model picker entry.
- No per-request model selection: `UTILITY_LLM_MODEL` is a deployment
  decision, unreachable from agent, session, or API input.
- No REST API endpoint for ad hoc utility LLM calls. System analysis tasks
  expose their own purpose-specific endpoints; the model call stays internal.
- No per-organization or per-session utility model configuration.
- No access from ordinary agent model selection or provider records.
