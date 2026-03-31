# Bashkit Requirements for Custom FileSystem Adapters

> **Status: IMPLEMENTED** - bashkit v0.1.12 exports all required types and
> `SessionFileSystemAdapter` is implemented in `crates/core/src/capabilities/virtual_bash.rs`.
>
> **Workspace Mount**: Session files are mounted at `/workspace` in the bash environment.
> Both `virtual_bash` and `session_file_system` capabilities normalize paths, enabling
> seamless file sharing between bash commands and file system tools.
>
> **Indexed Search**: `SessionFileSystemAdapter` implements bashkit's `SearchCapable` trait,
> delegating `grep -r` to `SessionFileStore::grep_files` for single-query database search
> instead of per-file linear scanning.

## Context

Everruns implements a custom `FileSystem` adapter that bridges bashkit to the session file store. This enables live visibility of files during bash execution - if another tool writes to the session filesystem while bash is running, those files are immediately visible.

The implementation is in `crates/core/src/capabilities/virtual_bash.rs` with `SessionFileSystemAdapter`.

## Indexed Search

Bashkit v0.1.12 restored `SearchCapable`/`SearchProvider` traits. When a `FileSystem` implements `SearchCapable`, builtins like `grep -r` use indexed search instead of reading every file individually.

`SessionFileSystemAdapter` implements `SearchCapable` by delegating to `SessionFileStore::grep_files()`, which executes a single SQL query with server-side regex matching. This eliminates N+1 database calls for recursive grep.

The sync-to-async bridge uses `std::thread::scope` with a dedicated thread and its own tokio runtime to avoid nesting `block_on` calls.

See `crates/core/src/capabilities/virtual_bash.rs` for the full implementation.

## Output Sanitization

`BashTool` sanitizes stdout/stderr before returning results to the LLM. This is the tool's responsibility — it calls `sanitize_exec_output()` from `crates/core/src/tool_output_sanitizer.rs`.

Pipeline: strip ANSI escape codes → collapse `\r`-overwritten lines → middle-truncate at 16 KiB (20% head / 80% tail). Stderr gets a smaller 4 KiB budget.

This reduces token waste from verbose build output (`cargo build`, `npm install`) by 40-60%. The EVE-225 hard limit (64 KiB) acts as a safety net for any tool that skips sanitization.

## Benefits

1. **Live file visibility** - Files written by other tools during bash execution are immediately visible
2. **No sync overhead** - Eliminates pre/post execution sync of entire filesystem
3. **Memory efficiency** - Files read on-demand instead of loading all into memory
4. **Consistency** - Single source of truth for file state
5. **Indexed search** - `grep -r` uses single-query database search instead of per-file reads
