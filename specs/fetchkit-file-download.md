# FetchKit File Download

File download support for the `web_fetch` capability via fetchkit's `FileSaver` trait.

## Design intent

fetchkit owns the `FileSaver` abstraction; consumers inject implementations:
- **CLI**: `LocalFileSaver` (real filesystem, ships with fetchkit)
- **Everruns**: `SessionFileSaver` adapter → `SessionFileStore` (per-session virtual filesystem)

This separation lets fetchkit stay environment-agnostic while everruns routes downloads through the session filesystem without granting host filesystem access.

## Key decisions

- **Binary encoding**: UTF-8 validity check (`std::str::from_utf8`) determines text vs base64 encoding — simpler and more correct than content-type heuristics
- **Schema opt-in**: `save_to_file` parameter hidden from LLM unless `enable_save_to_file(true)` — everruns enables it, other consumers can opt out
- **Binary content accepted**: `save_to_file` bypasses the normal binary rejection path in fetchkit's `DefaultFetcher`, allowing images/PDFs/archives to be downloaded
- **Context-aware**: `WebFetchTool.requires_context() = true` — file_store accessed from `ToolContext`. Without context, `save_to_file` is silently stripped (graceful degradation)

## Source files

- fetchkit `FileSaver` trait + `LocalFileSaver`: `fetchkit::file_saver` module
- `SessionFileSaver` adapter: `crates/core/src/capabilities/web_fetch.rs`
- `WebFetchTool` integration: same file

## Future: archive extraction (`FilesSaver`)

Planned extension: `FilesSaver` trait (extends `FileSaver`) with `save_and_extract()` for automatic unpacking of zip/tar.gz/tar archives after download. Separate trait so consumers opt-in. Not yet in fetchkit.
