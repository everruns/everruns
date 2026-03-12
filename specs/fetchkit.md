# fetchkit

External library ([github.com/everruns/fetchkit](https://github.com/everruns/fetchkit)) powering the `web_fetch` capability. Provides HTTP fetching, HTML-to-markdown conversion, SSRF protection, and file download.

## Integration

- `WebFetchTool` wraps `fetchkit::Tool` — delegates schema, description, llmtxt, and execution
- SSRF: `DnsPolicy::block_private_ips()` (default) blocks loopback, RFC1918, link-local, cloud metadata
- See `crates/core/src/capabilities/web_fetch.rs`

## File download (`FileSaver`)

fetchkit owns the `FileSaver` abstraction; consumers inject implementations:
- **CLI**: `LocalFileSaver` (real filesystem, ships with fetchkit)
- **Everruns**: `SessionFileSaver` adapter → `SessionFileStore` (per-session virtual filesystem)

Key decisions:
- **Binary encoding**: UTF-8 validity check (`std::str::from_utf8`) determines text vs base64 — simpler than content-type heuristics
- **Schema opt-in**: `save_to_file` hidden from LLM unless `enable_save_to_file(true)`
- **Binary content accepted**: `save_to_file` bypasses binary rejection in `DefaultFetcher`
- **Context-aware**: `WebFetchTool.requires_context() = true` — without context, `save_to_file` silently stripped

## Future: archive extraction (`FilesSaver`)

Planned: `FilesSaver` trait (extends `FileSaver`) with `save_and_extract()` for zip/tar.gz/tar. Separate trait, consumer opt-in. Not yet in fetchkit.
