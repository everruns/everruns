# Toolkit Library Contract

> Convention for external `*kit` libraries (bashkit, fetchkit, future kits) that expose tools for everruns integration.
> Not a shared crate — just a contract so all toolkit libraries feel the same to integrate.

## Problem

bashkit and fetchkit diverged in how they expose tool metadata and execution:

| Concern | bashkit | fetchkit |
|---------|---------|----------|
| Schema | Not exposed — wrapper hardcodes JSON | `tool.input_schema()` → `Value` |
| System prompt | `tool.system_prompt()` | `tool.llmtxt()` |
| Execution | Separate `Bash` struct, not on `Tool` | `tool.execute(req)` on `Tool` |
| Builder config | `.username()`, `.hostname()`, `.limits()`, `.env()` | `.enable_save_to_file()`, `.block_private_ips()` |

This means each new toolkit requires bespoke integration code in `crates/core/src/capabilities/`. The contract below standardizes the public API shape so the integration side is predictable.

## Contract overview

Three objects with clear responsibilities:

```
ToolBuilder (config)  →  Tool (metadata)  →  ToolExecution (runtime)
```

- **`ToolBuilder`** — kit-specific configuration, produces a `Tool`
- **`Tool`** — immutable metadata (name, description, schema, system prompt), produces a `ToolExecution`
- **`ToolExecution`** — stateful, single-use execution of one tool call

## Contract

### 1. `ToolBuilder`

Every `*kit` library exposes a `ToolBuilder` for configuration. The builder is a factory that can produce different artifacts depending on what the consumer needs.

```rust
let builder = mykit::ToolBuilder::new()
    .locale("en-US")
    .some_feature(true)
    .some_limit(1000);

// Full object — metadata + execution factory
let tool = builder.build();

// Or produce individual artifacts without building the full Tool:
let definition = builder.build_tool_definition();   // OpenAI function call JSON
let input_schema = builder.build_input_schema();     // JSON Schema for args
let output_schema = builder.build_output_schema();   // JSON Schema for result
let executor = builder.build_executor();             // generic Value → Value executor
```

Config methods are chainable. Build methods take `&self` (non-consuming) — you can call multiple on the same builder.

`ToolBuilder` need not be `Send + Sync`.

#### Required config methods

```rust
impl ToolBuilder {
    /// Set the locale for user-facing text (description, system prompt, error messages).
    /// BCP 47 language tag (e.g. "en-US", "uk-UA").
    /// Default: "en-US".
    fn locale(self, locale: &str) -> Self;
}
```

Kit-specific config methods are added alongside `locale()`.

#### Build methods

```rust
impl ToolBuilder {
    /// Build the full Tool (metadata + execution factory).
    fn build(&self) -> Tool;

    /// Build a standalone executor: accepts JSON args, returns JSON result.
    /// No metadata attached — useful for embedding in generic pipelines,
    /// test harnesses, or consumers that manage tool definitions separately.
    fn build_executor(&self) -> ToolExecutor;

    /// Build an OpenAI-compatible function tool definition.
    /// Returns JSON matching the OpenAI `tools` array element format:
    /// `{"type": "function", "function": {"name": "...", "description": "...", "parameters": {...}}}`
    fn build_tool_definition(&self) -> serde_json::Value;

    /// Build the JSON Schema for the tool's input parameters.
    /// Same as `Tool::input_schema()` but without building the full Tool.
    fn build_input_schema(&self) -> serde_json::Value;

    /// Build the JSON Schema for the tool's output.
    /// Describes the shape of `ToolOutput::result` so consumers can
    /// validate results or generate types.
    fn build_output_schema(&self) -> serde_json::Value;
}
```

**`ToolExecutor`** is a minimal, stateless executor:

```rust
pub struct ToolExecutor { /* internal */ }

impl ToolExecutor {
    /// Execute with JSON args, return JSON result.
    /// No ToolExecution indirection — fire-and-forget for simple consumers.
    async fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError>;

    /// Execute with an adapter.
    async fn execute_with<A: SomeAdapter>(
        &self,
        args: serde_json::Value,
        adapter: &A,
    ) -> Result<serde_json::Value, ToolError>;
}
```

**When to use which:**
- `build()` → everruns integration (full metadata + `ToolExecution` with cancel/stream)
- `build_executor()` → test harnesses, scripts, pipelines that just need `Value` in → `Value` out
- `build_tool_definition()` → registering tools with OpenAI-compatible APIs
- `build_input_schema()` / `build_output_schema()` → codegen, validation, documentation

### 2. `Tool` — metadata

`Tool` is immutable and holds all metadata baked in from the builder config:

```rust
impl Tool {
    /// Tool name for LLM invocation (e.g. "bash", "web_fetch").
    /// Snake_case, stable across versions.
    fn name(&self) -> &str;

    /// Human-readable display name for UI (e.g. "Bash", "Web Fetch").
    /// Localized per builder locale.
    fn display_name(&self) -> &str;

    /// Semantic version of the toolkit library (e.g. "0.1.8").
    /// Defaults to the crate version (`env!("CARGO_PKG_VERSION")`).
    /// Consumer can use this for diagnostics, logging, or compatibility checks.
    fn version(&self) -> &str;

    /// Human-readable description of the tool for the LLM.
    /// Baked in at build() time from builder config. Localized per builder locale.
    fn description(&self) -> &str;

    /// JSON Schema for the tool's input parameters.
    /// Must be a valid JSON Schema object with `"type": "object"`.
    /// Adapts to builder config (e.g. optional params appear/disappear).
    fn input_schema(&self) -> serde_json::Value;

    /// System prompt content (LLM instructions for using this tool).
    /// Returned as plain text — the consumer wraps it in XML tags.
    /// Returns empty string if no system prompt contribution.
    /// Localized per builder locale.
    fn system_prompt(&self) -> String;

    /// The locale this tool was built with (e.g. "en-US").
    fn locale(&self) -> &str;
}
```

`Tool` is `Send + Sync` and cheap to clone (typically `Arc` internals or static data).

**Rationale:** `input_schema()` was missing from bashkit, forcing the consumer to hardcode the schema and keep it in sync manually. All metadata methods must live on `Tool` so the consumer can delegate without duplication.

#### Locale affects

Locale controls the language of human-readable text:
- `description()` — tool description sent to LLM
- `display_name()` — UI label
- `system_prompt()` — LLM instructions
- Error messages from `ToolError` (user-facing variants)

Locale does **not** affect:
- `name()` — always English snake_case (LLM contract)
- `input_schema()` — property names and types are locale-independent
- `version()` — always semver

#### Examples

```rust
// English (default)
let tool = mykit::ToolBuilder::new().build();
assert_eq!(tool.name(), "web_fetch");
assert_eq!(tool.display_name(), "Web Fetch");
assert_eq!(tool.version(), "0.1.3");
assert!(tool.description().starts_with("Fetch content from a URL"));

// Ukrainian
let tool_ua = mykit::ToolBuilder::new().locale("uk-UA").build();
assert_eq!(tool_ua.name(), "web_fetch"); // unchanged
assert_eq!(tool_ua.display_name(), "Веб-завантаження");
assert!(tool_ua.description().starts_with("Завантажити вміст за URL"));
```

### 3. `ToolExecution` — runtime

`ToolExecution` represents a single in-flight tool call. Created from `Tool`, it is stateful and single-use.

```rust
impl Tool {
    /// Create an execution for the given arguments.
    /// Validates args against input_schema() before returning.
    fn execution(&self, args: serde_json::Value) -> Result<ToolExecution, ToolError>;
}
```

#### Required: `execute()`

```rust
impl ToolExecution {
    /// Run to completion. Consumes the execution.
    async fn execute(self) -> Result<ToolOutput, ToolError>;
}
```

If the tool needs an adapter (filesystem, file saver, etc.), provide a variant:

```rust
impl ToolExecution {
    /// Run to completion with an injected adapter.
    async fn execute_with<A: SomeAdapter>(self, adapter: &A) -> Result<ToolOutput, ToolError>;
}
```

The base `execute()` must work without adapters (returning an error if an adapter-dependent feature was requested in args). This lets the consumer call `execute()` in context-free paths and `execute_with()` when context is available.

#### Optional: cancellation

Toolkits with long-running operations (bash scripts, multi-step fetches) may support cancellation:

```rust
impl ToolExecution {
    /// Cancel the running execution.
    /// Returns partial results collected so far, or None if nothing was produced yet.
    async fn cancel(&self) -> Option<ToolOutput>;
}
```

Design rules:
- `cancel()` is safe to call multiple times (idempotent)
- `cancel()` is safe to call concurrently with `execute()` — it signals cancellation, `execute()` returns promptly
- Partial output follows the same `ToolOutput` shape (e.g. bashkit returns stdout collected so far, exit code -1)
- If the kit does not support cancellation, omit the method entirely — the consumer falls back to dropping the future

**bashkit example:** bash command running for 30s, user cancels at 5s. `cancel()` returns `ToolOutput { result: json!({"stdout": "partial output...", "exit_code": -1, "cancelled": true}), images: vec![] }`.

#### Optional: streaming

Toolkits that produce incremental output may expose a stream:

```rust
impl ToolExecution {
    /// Stream incremental output chunks during execution.
    /// The stream completes when execution finishes.
    /// Final result is still returned from execute().
    fn output_stream(&self) -> impl Stream<Item = ToolOutputChunk>;
}

pub struct ToolOutputChunk {
    /// Incremental content (e.g. stdout line, fetched bytes, progress update)
    pub data: serde_json::Value,
    /// Chunk type for consumer routing (e.g. "stdout", "stderr", "progress")
    pub kind: String,
}
```

Design rules:
- Streaming is opt-in — if the kit doesn't support it, omit the method
- `output_stream()` must be called **before** `execute()` — it returns a receiver, `execute()` drives the sender
- The stream is informational — the consumer uses it for live UI updates (e.g. streaming bash output to the terminal panel)
- The final `ToolOutput` from `execute()` is the authoritative result, not the concatenation of chunks
- If `cancel()` is called, the stream ends and `execute()` returns the partial result

**bashkit example:** streaming stdout/stderr line by line as the bash command runs.

**fetchkit example:** streaming download progress (`{"bytes_received": 1024, "total": 50000}`).

### 4. Adapter traits

When a toolkit needs a consumer-provided integration point (filesystem, file saver, HTTP client, etc.), it defines an adapter trait:

```rust
/// Trait name describes the capability, not the consumer.
/// e.g. `FileSaver`, `FileSystem` — not `SessionFileSaver`.
#[async_trait]
pub trait AdapterTrait: Send + Sync {
    async fn method(&self, ...) -> Result<..., AdapterError>;
}
```

Rules:
- Trait is defined in the toolkit crate, implemented by the consumer
- Trait is `Send + Sync` (adapters are shared across async tasks)
- Trait uses the toolkit's own error type or a dedicated adapter error type
- Toolkit may ship a default implementation for common cases (e.g. `LocalFileSaver` in fetchkit)

### 5. Error types

Every toolkit defines its own error enum:

```rust
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    /// Errors safe to show to the LLM (validation, not found, etc.)
    #[error("...")]
    UserFacing(String),

    /// Errors that should be hidden from the LLM (internal failures)
    #[error("...")]
    Internal(String),

    // Kit-specific variants are fine, but must be classifiable as
    // user-facing or internal via a method:
}

impl ToolError {
    /// Whether this error is safe to show to the LLM.
    fn is_user_facing(&self) -> bool;
}
```

The consumer maps these to `ToolExecutionResult::ToolError` or `ToolExecutionResult::InternalError` using `is_user_facing()`.

### 6. Output type

Execution returns a structured output:

```rust
pub struct ToolOutput {
    /// JSON result value
    pub result: serde_json::Value,
    /// Optional images (base64 + media type)
    pub images: Vec<ToolImage>,
}

pub struct ToolImage {
    pub base64: String,
    pub media_type: String,
}
```

If the tool never returns images, `images` is always empty. The consumer maps `ToolOutput` to `ToolExecutionResult::Success` or `ToolExecutionResult::SuccessWithImages`.

### 7. Re-exports

Toolkit crates re-export all types needed to implement adapter traits and handle results from `lib.rs`. Consumer code should only need `use mykit::{...}` — no reaching into submodules.

```rust
// mykit/src/lib.rs
pub use tool::{ToolBuilder, Tool, ToolExecutor, ToolExecution, ToolOutput, ToolOutputChunk, ToolImage};
pub use error::ToolError;
pub use adapters::{AdapterTrait, AdapterError, DefaultAdapter};
// Any types needed to implement AdapterTrait:
pub use adapters::{SaveResult, Metadata, DirEntry, ...};
```

### 8. No everruns dependency

Toolkit crates must not depend on `everruns-core` or any everruns crate. They are standalone libraries. The integration boundary is:

```
toolkit crate (standalone)       everruns-core             other consumers
├── ToolBuilder                  ├── XxxCapability          ├── build_executor()
│   ├── build() → Tool           │   └── builds Tool        │   └── Value in → Value out
│   ├── build_executor()         ├── XxxTool                ├── build_tool_definition()
│   ├── build_tool_definition()  │   ├── delegates metadata │   └── OpenAI function JSON
│   ├── build_input_schema()     │   ├── creates Execution  ├── build_input_schema()
│   └── build_output_schema()    │   ├── wires cancel/stream└── build_output_schema()
├── Tool (metadata+version)      │   └── maps to everruns
├── ToolExecutor (Value→Value)   └── AdapterImpl
├── ToolExecution (stateful)         └── bridges to SessionFileStore
│   ├── execute()
│   ├── cancel() [optional]
│   └── output_stream() [opt]
├── AdapterTrait
├── Error types
└── Output types
```

## Consumer integration pattern

With this contract, every capability wrapper follows the same structure:

```rust
// crates/core/src/capabilities/xxx.rs

use mykit;

pub struct XxxCapability;

impl Capability for XxxCapability {
    fn id(&self) -> &str { "xxx" }
    fn name(&self) -> &str { "Xxx" }
    fn description(&self) -> &str { "..." }

    fn tools_with_config(&self, config: &Value) -> Vec<Box<dyn Tool>> {
        vec![Box::new(XxxTool::new(config))]
    }

    fn system_prompt_preview(&self) -> Option<String> {
        let tool = mykit::ToolBuilder::new().some_feature(true).build();
        Some(tool.system_prompt())
    }

    async fn system_prompt_contribution_with_config(
        &self, _ctx: &SystemPromptContext, config: &Value,
    ) -> Option<String> {
        let tool = tool_from_config(config);
        Some(format!("<capability id=\"{}\">\n{}\n</capability>", self.id(), tool.system_prompt()))
    }
}

/// Helper: config JSON → built Tool
fn tool_from_config(config: &Value, locale: &str) -> mykit::Tool {
    let feature = config.get("some_feature").and_then(|v| v.as_bool()).unwrap_or(false);
    mykit::ToolBuilder::new()
        .locale(locale)
        .some_feature(feature)
        .build()
}

pub struct XxxTool {
    kit_tool: mykit::Tool,
}

impl XxxTool {
    fn new(config: &Value) -> Self {
        Self { kit_tool: tool_from_config(config) }
    }
}

#[async_trait]
impl Tool for XxxTool {
    fn name(&self) -> &str { self.kit_tool.name() }
    fn display_name(&self) -> Option<&str> { Some(self.kit_tool.display_name()) }
    fn description(&self) -> &str { self.kit_tool.description() }
    fn parameters_schema(&self) -> Value { self.kit_tool.input_schema() }

    async fn execute(&self, args: Value) -> ToolExecutionResult {
        let execution = match self.kit_tool.execution(args) {
            Ok(exec) => exec,
            Err(e) => return map_error(e),
        };
        match execution.execute().await {
            Ok(output) => map_output(output),
            Err(e) => map_error(e),
        }
    }

    async fn execute_with_context(&self, args: Value, ctx: &ToolContext) -> ToolExecutionResult {
        let execution = match self.kit_tool.execution(args) {
            Ok(exec) => exec,
            Err(e) => return map_error(e),
        };
        let adapter = MyAdapter::from_context(ctx);
        match execution.execute_with(&adapter).await {
            Ok(output) => map_output(output),
            Err(e) => map_error(e),
        }
    }
}

fn map_output(output: mykit::ToolOutput) -> ToolExecutionResult {
    if output.images.is_empty() {
        ToolExecutionResult::success(output.result)
    } else {
        ToolExecutionResult::success_with_images(output.result, /* map images */)
    }
}

fn map_error(e: mykit::ToolError) -> ToolExecutionResult {
    if e.is_user_facing() {
        ToolExecutionResult::tool_error(e.to_string())
    } else {
        ToolExecutionResult::internal_error_msg(e.to_string())
    }
}
```

## Current gaps

| Library | Gap | Migration |
|---------|-----|-----------|
| Library | Gap | Migration |
|---------|-----|-----------|
| both | No `locale()` on builder | Add; default `"en-US"`, thread through to description/system_prompt/errors |
| both | No `display_name()` on Tool | Add; return localized human-readable name |
| both | No `version()` on Tool | Add; return `env!("CARGO_PKG_VERSION")` |
| both | No `build_tool_definition()` | Add; return OpenAI function call JSON |
| both | No `build_output_schema()` | Add; describe shape of result JSON |
| both | No `build_executor()` | Add; return `ToolExecutor` for generic Value→Value usage |
| bashkit | No `ToolBuilder` — uses `BashTool::builder()` (naming) | Rename to `ToolBuilder` for consistency |
| bashkit | No `input_schema()` on Tool | Add method; remove hardcoded schema from `virtual_bash.rs` |
| bashkit | No `ToolExecution` — uses separate `Bash` struct | Wrap `Bash` in `ToolExecution`; `execute()` creates interpreter internally |
| bashkit | `system_prompt()` naming OK — fetchkit uses `llmtxt()` | Standardize on `system_prompt()` for both |
| bashkit | No `name()` on Tool | Add; return `"bash"` |
| bashkit | No structured `ToolOutput` — returns raw stdout/stderr | Wrap in `ToolOutput { result: json!({...}), images: vec![] }` |
| bashkit | Has natural cancel/stream support (interpreter is stateful) | Expose via `ToolExecution::cancel()` and `output_stream()` |
| fetchkit | `Tool::builder()` naming OK but returns unnamed builder type | Expose as `ToolBuilder` |
| fetchkit | Uses `llmtxt()` instead of `system_prompt()` | Rename (or alias) to `system_prompt()` |
| fetchkit | `execute()` takes `FetchRequest`, not `Value` | Parse `Value` → `FetchRequest` inside `ToolExecution::execute()` |
| fetchkit | Error enum doesn't have `is_user_facing()` | Add method; currently all errors are user-facing |
| fetchkit | No `name()` on Tool | Add; return `"web_fetch"` |
| fetchkit | No structured `ToolOutput` | Wrap response in `ToolOutput` |
| fetchkit | No `ToolExecution` — `execute()` lives on `Tool` | Move to `ToolExecution`; streaming progress is a natural fit |

## Non-goals

- **Shared trait crate**: No `toolkit-common` dependency. Each kit defines its own types following the same shape. This avoids coupling kit release cycles.
- **Generic tool registration**: The consumer still writes a thin `XxxCapability` + `XxxTool` wrapper. The contract just makes that wrapper predictable and mechanical.
- **Versioning contract**: Kits version independently. Breaking changes to the contract surface are communicated via this spec, not semver of a shared crate.
