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

## Contract

### 1. `ToolBuilder` — the primary API for everruns

The `ToolBuilder` is the main integration surface. everruns constructs tool builders (often at init time), configures them, and calls metadata/execution methods directly on the builder or on the `Tool` it produces.

Every `*kit` library exposes a `ToolBuilder` with kit-specific config methods:

```rust
let builder = mykit::ToolBuilder::new()
    // kit-specific config methods
    .some_feature(true)
    .some_limit(1000);

// Metadata is available on the builder (no need to build first)
let schema = builder.input_schema();
let prompt = builder.system_prompt();

// Build produces an immutable, executable Tool
let tool = builder.build();
```

`ToolBuilder` methods are chainable. `build()` consumes the builder and returns a `Tool`.

Both `ToolBuilder` and `Tool` are `Send + Sync`.

### 2. Tool metadata methods

Available on both `ToolBuilder` (for pre-build introspection) and `Tool` (post-build):

```rust
// On ToolBuilder and Tool
impl {
    /// Tool name for LLM invocation (e.g. "bash", "web_fetch").
    /// Snake_case, stable across versions.
    fn name(&self) -> &str;

    /// Human-readable description of the tool for the LLM.
    /// May vary based on builder config (e.g. enabled features change the description).
    fn description(&self) -> String;

    /// JSON Schema for the tool's input parameters.
    /// Must be a valid JSON Schema object with `"type": "object"`.
    /// Adapts to builder config (e.g. optional params appear/disappear).
    fn input_schema(&self) -> serde_json::Value;

    /// System prompt content (LLM instructions for using this tool).
    /// Returned as plain text — the consumer wraps it in XML tags.
    /// Returns empty string if no system prompt contribution.
    fn system_prompt(&self) -> String;
}
```

**Why on builder too:** everruns needs metadata at capability-collection time to generate system prompts and tool definitions. The builder config affects metadata (e.g. `enable_save_to_file` changes the schema and description). Having metadata on the builder avoids building a throwaway `Tool` just to read the schema.

**Rationale:** `input_schema()` was missing from bashkit, forcing the consumer to hardcode the schema and keep it in sync manually. All metadata methods must live on the builder/tool so the consumer can delegate without duplication.

### 3. `Tool` — execution

`Tool` is the built, immutable, executable artifact. Every `Tool` provides an async execute method:

```rust
impl Tool {
    /// Execute the tool with JSON arguments matching `input_schema()`.
    /// Returns a kit-specific result type (see §5).
    async fn execute(&self, args: serde_json::Value) -> Result<ToolOutput, ToolError>;
}
```

If the tool needs an adapter (filesystem, file saver, etc.), provide a second method:

```rust
impl Tool {
    /// Execute with an injected adapter.
    async fn execute_with<A: SomeAdapter>(
        &self,
        args: serde_json::Value,
        adapter: &A,
    ) -> Result<ToolOutput, ToolError>;
}
```

The base `execute()` must work without adapters (returning an error or degraded result if the adapter-dependent feature is requested). This lets the consumer call `execute()` in context-free paths and `execute_with()` when context is available.

**bashkit note:** bashkit currently uses a separate `Bash` interpreter struct for execution. Under this contract, `Tool::execute()` would create the interpreter internally. Per-execution config (filesystem adapter, working directory) flows through the args or through `execute_with()`.

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
pub use tool::{ToolBuilder, Tool, ToolOutput, ToolImage};
pub use error::ToolError;
pub use adapters::{AdapterTrait, AdapterError, DefaultAdapter};
// Any types needed to implement AdapterTrait:
pub use adapters::{SaveResult, Metadata, DirEntry, ...};
```

### 8. No everruns dependency

Toolkit crates must not depend on `everruns-core` or any everruns crate. They are standalone libraries. The integration boundary is:

```
toolkit crate (standalone)     everruns-core
├── ToolBuilder (config+meta)  ├── XxxCapability (implements Capability trait)
├── Tool (execute)             │   ├── uses ToolBuilder for metadata + system prompt
├── AdapterTrait               │   └── builds Tool for execution
├── Error types                ├── XxxTool (implements everruns Tool trait)
└── Output types               │   ├── delegates metadata to toolkit ToolBuilder/Tool
                               │   ├── delegates execution to toolkit Tool
                               │   └── maps errors/output to ToolExecutionResult
                               └── AdapterImpl (implements toolkit::AdapterTrait)
                                   └── bridges to SessionFileStore, etc.
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
        // Builder metadata without building a Tool — preview with all features enabled
        Some(mykit::ToolBuilder::new().some_feature(true).system_prompt())
    }

    async fn system_prompt_contribution_with_config(
        &self, _ctx: &SystemPromptContext, config: &Value,
    ) -> Option<String> {
        // Builder reads config, generates config-aware system prompt
        let builder = builder_from_config(config);
        Some(format!("<capability id=\"{}\">\n{}\n</capability>", self.id(), builder.system_prompt()))
    }
}

/// Helper: config JSON → ToolBuilder
fn builder_from_config(config: &Value) -> mykit::ToolBuilder {
    let feature = config.get("some_feature").and_then(|v| v.as_bool()).unwrap_or(false);
    mykit::ToolBuilder::new().some_feature(feature)
}

pub struct XxxTool {
    kit_tool: mykit::Tool,
    description: String,
}

impl XxxTool {
    fn new(config: &Value) -> Self {
        let builder = builder_from_config(config);
        let description = builder.description();
        let kit_tool = builder.build();
        Self { kit_tool, description }
    }
}

#[async_trait]
impl Tool for XxxTool {
    fn name(&self) -> &str { self.kit_tool.name() }
    fn description(&self) -> &str { &self.description }
    fn parameters_schema(&self) -> Value { self.kit_tool.input_schema() }

    async fn execute(&self, args: Value) -> ToolExecutionResult {
        match self.kit_tool.execute(args).await {
            Ok(output) => ToolExecutionResult::success(output.result),
            Err(e) if e.is_user_facing() => ToolExecutionResult::tool_error(e.to_string()),
            Err(e) => ToolExecutionResult::internal_error_msg(e.to_string()),
        }
    }

    async fn execute_with_context(&self, args: Value, ctx: &ToolContext) -> ToolExecutionResult {
        let adapter = MyAdapter::from_context(ctx);
        match self.kit_tool.execute_with(args, &adapter).await {
            Ok(output) => map_output(output),
            Err(e) => map_error(e),
        }
    }
}
```

## Current gaps

| Library | Gap | Migration |
|---------|-----|-----------|
| bashkit | No `ToolBuilder` — uses `BashTool::builder()` (naming) | Rename to `ToolBuilder` for consistency |
| bashkit | No `input_schema()` on builder or Tool | Add method; remove hardcoded schema from `virtual_bash.rs` |
| bashkit | No `execute()` on Tool — uses separate `Bash` struct | Add `execute()` / `execute_with()` that wraps `Bash` internally |
| bashkit | `system_prompt()` naming OK — fetchkit uses `llmtxt()` | Standardize on `system_prompt()` for both |
| bashkit | No `name()` on builder or Tool | Add; return `"bash"` |
| bashkit | No structured `ToolOutput` — returns raw stdout/stderr | Wrap in `ToolOutput { result: json!({...}), images: vec![] }` |
| fetchkit | `Tool::builder()` naming OK but returns unnamed builder type | Expose as `ToolBuilder` |
| fetchkit | Uses `llmtxt()` instead of `system_prompt()` | Rename (or alias) to `system_prompt()` |
| fetchkit | `execute()` takes `FetchRequest`, not `Value` | Add `Value`-based overload or keep `FetchRequest` and document parse step |
| fetchkit | Error enum doesn't have `is_user_facing()` | Add method; currently all errors are user-facing |
| fetchkit | No `name()` on builder or Tool | Add; return `"web_fetch"` |
| fetchkit | No structured `ToolOutput` | Wrap response in `ToolOutput` |

## Non-goals

- **Shared trait crate**: No `toolkit-common` dependency. Each kit defines its own types following the same shape. This avoids coupling kit release cycles.
- **Generic tool registration**: The consumer still writes a thin `XxxCapability` + `XxxTool` wrapper. The contract just makes that wrapper predictable and mechanical.
- **Versioning contract**: Kits version independently. Breaking changes to the contract surface are communicated via this spec, not semver of a shared crate.
