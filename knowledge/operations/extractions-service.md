---
type: Specification
title: Native extraction services
description: Typed extraction belongs beside decisions and chat, with native evidence semantics and organization-owned credentials.
tags: [providers, framework, server, extractions]
---

# Native extraction services

## Decision

Treat extraction as its own service, not a prompt template over chat. The neutral
contract lives in `crates/contracts/src/extractions.rs`; native transport and
model discovery live in `crates/drivers/drivers/src/extractions/`. The Framework
facade and Server share the contract. The upstream OpenRouter surface is alpha.

Preserve absence, guesses, confidence, source evidence, truncation and usage.
Missing confidence is unknown, not zero or one. Span offsets count Unicode code
points and may refer to returned normalized state. Do not reinterpret estimates
or forecasts as observed facts. Validate typed values and evidence before return.

## Provider boundaries

OpenRouter has its own extraction endpoint and discovery endpoint filter. Direct
Cloudflare uses Workers AI Clairic, with its narrower options and asset semantics.
Inception and ScaleDown remain reachable through OpenRouter; their direct APIs
have different request/result shapes and need dedicated adapters before claiming
support. Do not silently route a native extraction request through chat.

A provider/model identity may support both chat and extraction. Coalesce discovery
by model ID and union service capabilities while retaining existing chat metadata;
do not create duplicate catalog identities or overwrite chat profiles.

## Ownership and admission

The Server resolves only the authenticated organization's enabled configured
model, active provider, and stored credentials. Require model-management rights,
reject read-only callers and host-managed providers until hosted extraction
accounting exists. Do not borrow deployment utility credentials for tenant calls.
The explicit deployment-owned utility is opt-in by extraction model configuration.

Bound input assets before decoding and responses before parsing. Disable redirects
and suppress provider response bodies in errors: they may repeat source documents
or credentials. No remote asset URL fetching occurs in this API.

## Markkit composition

Keep Markkit's local document conversion independent of provider credentials and
inference. Convert and review locally, then explicitly send the reviewed Markdown
to the extraction service. The input-file Framework example demonstrates this
boundary without adding a provider dependency to Markkit.

## Qualification

Offline contract, mock transport and org-bound API tests do not establish vendor
availability, pricing or accuracy. The explicit live qualification executable uses synthetic invoices and
require explicit opt-in and credentials; paid qualification is a separate action.
See `docs/framework/extractions.md` for supported surfaces and operational limits.
