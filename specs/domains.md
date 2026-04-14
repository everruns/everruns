# Domain Modules

Feature-oriented modules that own request types, validation, business logic,
and shared query helpers for a single domain. Both HTTP and MCP adapters
dispatch to commands defined here — no separate "services" layer.

## Motivation

Before this pattern, a single domain (e.g. agents) was scattered across four
locations: `api/agents.rs` (HTTP handlers + request types + validation),
`services/agent.rs` (business logic + policy), `api/mcp_endpoint/handlers.rs`
(MCP handlers), and `api/mcp_endpoint/catalog.rs` (static catalog entries).
Adding a field meant touching 4+ files in 3 directories. Validation ran only
in the HTTP path — MCP callers bypassed it. The static catalog duplicated
metadata already present in utoipa annotations.

## Module Structure

```
crates/server/src/domains/
  mod.rs              ← pub mod agents; pub mod sessions; ...
  common.rs           ← Command trait, CommandError, CommandContext, Paginated

  agents/
    mod.rs            ← re-exports
    commands.rs       ← CreateAgent, ListAgents, GetAgent, ...
    queries.rs        ← resolve(), row_to_agent(), ensure_name_available()
    types.rs          ← request/response types owned by this domain
  sessions/
    ...
```

Each domain directory contains three files:

| File | Purpose | Who calls it |
|------|---------|--------------|
| `commands.rs` | User-facing operations (validation + policy + persistence) | HTTP adapters, MCP dispatch |
| `queries.rs` | Shared read/write helpers (no policy, no input validation) | Commands, gRPC worker, other domains |
| `types.rs` | Request/response types, DB row types owned by this domain | Commands, queries, HTTP adapters |

## Command Trait

See `crates/server/src/domains/common.rs` for the canonical definition.

```rust
pub trait Command: DeserializeOwned + Send + 'static {
    type Output: Serialize + Send;

    /// Static metadata — drives MCP catalog generation.
    fn meta() -> CommandMeta;

    /// Policy checked before execute(). None = no auth required.
    fn policy() -> Option<&'static Policy> { None }

    /// Execute the command. All validation and business logic lives here.
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError>;
}
```

### CommandMeta

```rust
pub struct CommandMeta {
    pub name: &'static str,        // "create_agent"
    pub category: &'static str,    // "agents"
    pub description: &'static str, // Human-readable, shown in MCP catalog
    pub method: &'static str,      // "POST" — HTTP method (metadata only)
    pub path: &'static str,        // "/v1/agents" — HTTP path (metadata only)
}
```

### CommandError

Protocol-agnostic error type. HTTP adapters convert to `(StatusCode, Json<ErrorResponse>)`;
MCP dispatch calls `.to_string()`.

```rust
pub enum CommandError {
    BadRequest(String),     // 400
    Forbidden(String),      // 403
    NotFound(String),       // 404
    Conflict(String),       // 409
    Internal(anyhow::Error), // 500
}
```

### Ctx (CommandContext)

Shared execution context. Constructed once, cloned into both adapters.

```rust
pub struct Ctx {
    pub caller: Caller,
    pub db: Arc<StorageBackend>,
    // Domain-specific helpers that multiple commands need:
    pub capability_service: Arc<CapabilityService>,
    pub encryption: Option<Arc<EncryptionService>>,
}
```

`Ctx` holds only cross-cutting dependencies (DB, encryption, capability registry).
Domain-specific helpers live as free functions in `queries.rs`, not as service fields.

## Inventory Registration

Commands self-register via `inventory::submit!`. The MCP catalog and dispatch
table are generated at runtime from the registry — no hand-maintained
`CATALOG` array or `resolve_handler` match.

```rust
pub struct CommandDescriptor {
    pub meta: fn() -> CommandMeta,
    pub dispatch: for<'a> fn(
        serde_json::Value,
        &'a Ctx,
    ) -> Pin<Box<dyn Future<Output = Result<String, CommandError>> + Send + 'a>>,
}

inventory::collect!(CommandDescriptor);
```

Adding a new command: implement `Command` for a struct, add `inventory::submit!`
at the bottom. The catalog auto-updates. No other files to touch.

## Writing a Command

Canonical pattern (see `domains/agents/commands.rs` for reference):

```rust
#[derive(Debug, Deserialize)]
pub struct CreateAgent {
    pub name: String,
    pub system_prompt: String,
    // ... fields are the public API contract
}

impl Command for CreateAgent {
    type Output = Agent;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_agent",
            category: "agents",
            description: "Create a new agent with a name, system prompt, and optional capabilities.",
            method: "POST",
            path: "/v1/agents",
        }
    }

    fn policy() -> Option<&'static Policy> { Some(&AGENT_MANAGE) }

    async fn execute(self, ctx: &Ctx) -> Result<Agent, CommandError> {
        // 1. Validate input
        // 2. Business rules (name uniqueness, capability refs, etc.)
        // 3. Persist (DB calls via ctx.db or queries::*)
        // 4. Return domain type
    }
}
inventory::submit! { CommandDescriptor::of::<CreateAgent>() }
```

### Command composition

Commands can call other commands directly. No need for service indirection:

```rust
impl Command for CopyAgent {
    async fn execute(self, ctx: &Ctx) -> Result<Agent, CommandError> {
        let source = queries::resolve(&ctx.db, ctx.org_id(), &self.id).await?
            .ok_or_else(|| CommandError::NotFound("Agent not found".into()))?;
        CreateAgent { name: copy_name, ..from_source }.execute(ctx).await
    }
}
```

## Writing Queries

Queries are free functions, not methods on a service struct. No policy checks,
no input validation. Used by commands (user-facing) and gRPC worker (internal).

```rust
// domains/agents/queries.rs

pub fn row_to_agent(row: AgentRow, caps: Vec<AgentCapabilityConfig>) -> Agent { ... }
pub async fn get_capabilities(db: &StorageBackend, uuid: Uuid) -> Result<Vec<AgentCapabilityConfig>> { ... }
pub async fn resolve(db: &StorageBackend, org_id: i64, id_or_name: &str) -> Result<Option<Agent>> { ... }
pub async fn ensure_name_available(db: &StorageBackend, org_id: i64, name: &str, exclude: Option<AgentId>) -> Result<(), CommandError> { ... }
```

## HTTP Adapters

Thin wrappers. Axum extractors → command struct → execute → wrap response.
OpenAPI annotations stay here. ~5 lines per handler.

```rust
// api/agents.rs
pub async fn create(org: ResolvedOrg, State(s): State<AppState>, Json(cmd): Json<CreateAgent>) -> ApiResult<Agent> {
    let agent = cmd.execute(&s.ctx(&org)).await?;
    Ok((StatusCode::CREATED, Json(s.wrap(agent))))
}
```

`impl From<CommandError> for (StatusCode, Json<ErrorResponse>)` provides
automatic HTTP error mapping.

## MCP Dispatch

Generic dispatch replaces hand-written handlers. The entire `handlers.rs` and
static `CATALOG` array collapse to:

```rust
// Built once at startup from inventory
pub fn catalog() -> Vec<Operation> {
    inventory::iter::<CommandDescriptor>().map(|d| (d.meta)().into()).collect()
}

pub async fn dispatch(name: &str, params: Value, ctx: &Ctx) -> Result<String, String> {
    DISPATCH_TABLE.get(name)
        .ok_or_else(|| format!("Unknown command: {name}"))?
        .dispatch(params, ctx).await
        .map_err(|e| e.to_string())
}
```

## Migration Strategy

Domains are migrated incrementally. During migration, both the old service and
the new command can coexist: the service delegates to queries, the command owns
the logic. Once all callers are migrated, the service file is removed.

### Migration order

1. `domains/common.rs` — infrastructure
2. `agents` — canonical reference implementation
3. `harnesses`, `apps`, `mcp_servers`, `agent_identities`, `skills`, `capabilities`
4. Wire MCP dispatch to inventory
5. Remove empty service files

### When NOT to use this pattern

- **Internal-only logic** with no user-facing API surface (e.g. `llm_resolver`,
  `model_sync`, `usage_tracking`). Keep these as standalone services/modules.
- **Cross-cutting listeners** (e.g. `NotificationEventListener`,
  `UsageTrackingListener`). These are event-driven, not command-driven.
- **Complex orchestration** that spans multiple domains (e.g. session creation
  resolving agent + harness + capabilities). These remain in their domain's
  commands but may call other domains' queries.

## Testing

- Commands are tested directly: construct `Ctx` with a test DB, call `execute()`.
- Queries are tested as unit functions: pass a `StorageBackend`, assert results.
- HTTP adapters: integration tests via `axum::test` (unchanged).
- MCP dispatch: test via `dispatch("create_agent", json!({...}), &ctx)`.

## Checklist for adding a new domain

1. Create `domains/{name}/` with `mod.rs`, `commands.rs`, `queries.rs`, `types.rs`
2. Move request types from `api/{name}.rs` to `types.rs`
3. Move business logic from `services/{name}.rs` to `commands.rs`
4. Extract shared DB helpers to `queries.rs`
5. Each command: `inventory::submit! { CommandDescriptor::of::<Cmd>() }`
6. Update `api/{name}.rs` to call commands (thin adapter)
7. Remove old service file once all callers migrated
8. Verify `just pre-push` passes
