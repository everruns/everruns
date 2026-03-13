# fetchkit

External library ([github.com/everruns/fetchkit](https://github.com/everruns/fetchkit)) powering the `web_fetch` capability. Provides HTTP fetching, HTML-to-markdown conversion, SSRF protection, and file download.

## Integration

- `WebFetchCapability` uses `fetchkit::ToolBuilder` to configure the tool
- `WebFetchTool` wraps `fetchkit::Tool` — delegates schema, description, llmtxt, and execution
- All metadata (description, system prompt, input schema) comes from `fetchkit::ToolBuilder`, not constants
- SSRF: `DnsPolicy::block_private_ips()` (default) blocks loopback, RFC1918, link-local, cloud metadata
- See `crates/core/src/capabilities/web_fetch.rs`

## File download (`FileSaver`)

fetchkit owns the `FileSaver` abstraction; consumers inject implementations:
- **CLI**: `LocalFileSaver` (real filesystem, ships with fetchkit)
- **Everruns**: `SessionFileSaver` adapter → `SessionFileStore` (per-session virtual filesystem)

Key decisions:
- **Config-gated**: file download enabled via per-capability config `{"enable_file_download": true}` — harnesses/agents opt in
- **ToolBuilder-driven**: `enable_save_to_file` on ToolBuilder controls schema, description, and system prompt content
- **Binary encoding**: UTF-8 validity check (`std::str::from_utf8`) determines text vs base64 — simpler than content-type heuristics
- **Binary content accepted**: `save_to_file` bypasses binary rejection in `DefaultFetcher`

## Capability config mechanism

`WebFetchCapability` implements `tools_with_config` and `system_prompt_contribution_with_config` on the `Capability` trait. These methods read the per-capability config JSON during capability collection, enabling file download when `enable_file_download: true` is set. Generic and Chat harnesses set this config alongside `session_file_system`.

## Future: archive extraction (`FilesSaver`)

Planned: `FilesSaver` trait (extends `FileSaver`) with `save_and_extract()` for zip/tar.gz/tar. Separate trait, consumer opt-in. Not yet in fetchkit.
