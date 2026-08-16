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
- `everruns-host` owns the concrete OpenAI implementation behind its optional
  `utility-openai` feature; the utility LLM service remains the
  capability-facing typed API.
- Utility LLM provider transport is host-owned. It does not route through
  `EgressService` and is not governed by tenant/agent egress policy such as
  `EVERRUNS_SYSTEM_ALLOWLIST_ENABLED`.

The model is hardcoded to `gpt-5.5`. Requests do not expose tools, tool search,
previous response IDs, model overrides, or provider credentials. By default the
service sends no reasoning parameter; callers can opt into `low`, `medium`, or
`high`.

## System Configuration

The service is configured from process environment:

- `UTILITY_OPENAI_API_KEY`

When the variable is unset or empty, the service is disabled. Disabled
deployments should call `is_configured()` before attempting optional utility
work, or handle the configuration error returned by completion methods.

The default server and worker platform profiles enable `everruns-host`'s
`utility-openai` feature and resolve `SystemUtilityLlmConfig::from_env()` during
platform construction. Embedders can bypass env-based setup by constructing a
custom `HostComposition` and calling
`HostComposition::builder().utility_llm_service(...)` without enabling the
concrete implementation.

## Non-Goals

- No user-facing model picker entry.
- No REST API endpoint for ad hoc utility LLM calls. System analysis tasks
  expose their own purpose-specific endpoints; the model call stays internal.
- No per-organization or per-session utility model configuration.
- No access from ordinary agent model selection or provider records.
