---
type: Specification
title: "fetchkit"
description: "fetchkit library powering the `web_fetch` capability."
tags:
  - everruns
  - execution
---
# fetchkit

External library ([github.com/everruns/fetchkit](https://github.com/everruns/fetchkit)) powering the `web_fetch` capability. Provides HTTP fetching, focused content extraction, bounded crawl discovery, HTML-to-markdown conversion, SSRF protection, file download, and opt-in lightweight rendering.

## Integration

- `WebFetchCapability` uses `fetchkit::ToolBuilder` to configure the tool
- `WebFetchTool` wraps `fetchkit::Tool`, delegates schema, description, llmtxt, and execution
- All metadata (description, system prompt, input schema) comes from `fetchkit::ToolBuilder`, not constants
- The wrapper deserializes FetchKit's request contract so content focus,
  conditional fetch, bounded crawl, and rendering inputs reach the library
  without a second field-by-field contract that can drift.
- **Egress path (default in the runtime)**: when `ToolContext.egress_service`
  is present, fetchkit's `HttpTransport` (fetchkit >= 0.4) is injected via
  `ToolBuilder::transport` with `EgressHttpTransport`
  (`crates/integrations/src/web_fetch/egress_transport.rs`). fetchkit keeps the
  whole pipeline, specialized fetchers (GitHub, Wikipedia, arXiv, …), SSRF
  via `DnsPolicy` (resolve-then-check; pinned addresses forwarded to
  `EgressRequest.pinned_addrs`), per-hop redirect validation, body caps,
  while every HTTP hop crosses the egress boundary, which enforces the
  network access list and system allowlist and signs the hop.
- **Direct path** (no egress service in context, e.g. embedded hosts):
  fetchkit owns transport; SSRF via `DnsPolicy::block_private_ips()` (blocks
  loopback, RFC1918, link-local, cloud metadata). Crawl requests are rejected
  when a network access list or system allowlist is active because this path
  cannot re-check policy for discovered pages.
- The system allowlist and network access list are pre-checked on the initial
  URL in the tool for clear user-facing errors on both paths.
- See `crates/integrations/src/web_fetch/lib.rs`

## Agent-oriented fetching

FetchKit owns the structured fetchers, focused extraction, page quality signals,
and crawl result model. Everruns exposes those inputs and returns the upstream
response unchanged:

- `content_focus: "agent"` selects FetchKit's best low-noise extraction strategy.
- `crawl: true` discovers a bounded, same-origin page set; `max_pages` is bounded
  by FetchKit's schema and implementation.
- Conditional request inputs and response metadata allow callers to avoid
  refetching unchanged content.
- Specialized fetchers return compact structured content for supported source
  types; unsupported URLs continue through the default fetcher.

## Rendered fetch

The `render-rakers` Cargo feature is compiled in and the backend is enabled on
the tool, but each request must explicitly pass `render: "rakers"`. This is
lightweight JavaScript/DOM execution for pages whose useful content is produced
inline; it is not a full browser.

Rendered fetch preserves the normal initial-request URL, DNS, egress, timeout,
and body-size policy. FetchKit denies renderer-initiated subresource requests,
caps rendered output before conversion, and applies a per-script execution
timeout. This prevents rendered HTML from creating a second path around the
host egress boundary.

## File download (`FileSaver`)

fetchkit owns the `FileSaver` abstraction; consumers inject implementations:
- **CLI**: `LocalFileSaver` (real filesystem, ships with fetchkit)
- **Everruns**: `SessionFileSaver` adapter → `SessionFileSystem` (per-session virtual filesystem)

Key decisions:
- **Config-gated**: file download enabled via per-capability config `{"enable_file_download": true}`, harnesses/agents opt in
- **ToolBuilder-driven**: `enable_save_to_file` on ToolBuilder controls schema, description, and system prompt content
- **Binary encoding**: UTF-8 validity check (`std::str::from_utf8`) determines text vs base64, simpler than content-type heuristics
- **Binary content accepted**: `save_to_file` bypasses binary rejection in `DefaultFetcher`

## Capability config mechanism

`WebFetchCapability` implements `tools_with_config` and `system_prompt_contribution_with_config` on the `Capability` trait. These methods read the per-capability config JSON during capability collection, enabling file download when `enable_file_download: true` is set. Generic and Chat harnesses set this config alongside `session_file_system`.

## Request signing

fetchkit's own `bot-auth` feature is off. `web_fetch` hops ask the egress
boundary for `EgressSigning::PlatformDefault`, and the egress service signs
them with Web Bot Auth, one hop at a time, so every signing caller shares one
key and one format. See "Signing" in
[`knowledge/operations/egress.md`](../operations/egress.md).

## Future: archive extraction (`FilesSaver`)

Planned: `FilesSaver` trait (extends `FileSaver`) with `save_and_extract()` for zip/tar.gz/tar. Separate trait, consumer opt-in. Not yet in fetchkit.
