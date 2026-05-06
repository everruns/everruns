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

User-facing domains normally expose `commands.rs`, `queries.rs`, and `types.rs`.
Some internal-only domains exist purely to house shared orchestration, in which
case they may only export `service.rs` (or another helper module) until they
gain user-facing commands. In both cases, the helper code stays under the
owning domain rather than a global business-logic layer.

The standard files are:

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

    /// Policy checked by `run()` before `execute()`. None = public/no auth.
    fn policy() -> Option<&'static Policy> { None }

    /// Execute the command. All validation and business logic lives here.
    /// SECURITY: Do not call directly from HTTP/MCP/gRPC adapters — it skips
    /// the policy check. Use `run()` instead. `execute` stays public so one
    /// command can compose another already-authorized command.
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError>;

    /// Public entry point. Every caller (HTTP adapter, MCP dispatch, gRPC
    /// `ExecuteCommand`, platform capability) goes through here.
    ///
    /// Evaluates `policy()` against `ctx.permission_resolver` (so SaaS
    /// custom resolvers apply), then delegates to `execute`.
    async fn run(self, ctx: &Ctx) -> Result<Self::Output, CommandError>;
}
```

### Enforcement contract

Policy enforcement is performed **once**, at `Command::run`. Every caller kind (HTTP, MCP, gRPC `ExecuteCommand`, platform capability) routes through `run`, which evaluates `Command::policy()` against the `PermissionResolver` carried in `Ctx`. See `specs/permissions.md` for the full model.

- HTTP adapters: `CreateAgent(req).run(&ctx).await` (never `.execute(...)`).
- MCP and gRPC `ExecuteCommand` use `domains::common::dispatch()`, which calls `run` internally.
- Every non-GET command must declare `policy()`; `crates/server/tests/command_policy_enforcement_test.rs` verifies this at test time by iterating `inventory::iter::<CommandDescriptor>`.
- `Ctx::new` requires the resolver as its fifth argument, so constructing a `Ctx` without a resolver is a compile error. Tests use `Ctx::minimal_for_test` which defaults to `DefaultPermissionResolver`.

`Command::run` is also the chokepoint for protocol-agnostic cross-cutting concerns (tracing span, metrics, structured failure log) — every caller that goes through `run` is instrumented. The HTTP `Dispatcher` (`crates/server/src/api/dispatch.rs`) is the matching HTTP-only chokepoint: it bundles `(Ctx, UrlBuilder)` per request and exposes `run_*` helpers (`run`, `run_with_urls`, `run_created`, `run_created_with_urls`, `run_no_content`, `run_list`, `run_list_with_urls`, `run_paginated_with_urls`) so trivial HTTP handlers collapse to `state.dispatcher(&org).run_*(cmd).await`. Future HTTP-only cross-cutting concerns (request_id propagation, content-type negotiation, response body decoration) belong here once they are generalised — today most of those concerns are still threaded through specialised handlers. See `specs/observability.md` and `specs/prometheus-metrics.md` for the metric contract.

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
    pub positional_arg: fn() -> Option<&'static str>,
    pub dispatch: for<'a> fn(
        serde_json::Value,
        &'a Ctx,
    ) -> Pin<Box<dyn Future<Output = Result<String, CommandError>> + Send + 'a>>,
}

inventory::collect!(CommandDescriptor);
```

Adding a new command: implement `Command` for a struct, add `inventory::submit!`
at the bottom. The catalog auto-updates. No other files to touch.

### MCP `query`/`execute` positional args

`Command::positional_arg()` lets a command opt in to accepting one positional
argument under MCP's scripted MCP tools (`query` and `execute`, both backed by
bashkit). Returning `Some("id")` on
`get_agent` means callers can write `get_agent <id>` instead of
`get_agent --id <id>`; the mcp_endpoint rewrites the command string at
statement boundaries before bashkit's flag parser runs. Only opt in when the
command has exactly one required primary-key-like parameter. See EVE-323.

`Command::read_only()` controls whether the command is exposed through the
read-only MCP `query` tool. The default is GET-only; override it for safe
POST-style commands such as previews or structured search helpers.

`Command::output_schema()` and `Command::output_shape()` feed MCP `discover`.
Every command reports a valid output schema; commands whose output shape is
important for jq scripting should override both. Use `output_shape = "paginated"`
for `{data,total,offset,limit}` list responses and `output_shape = "array"` for
bare array responses.

### Array and object parameters in bashkit builtins

`api/mcp_endpoint/catalog.rs::bashkit_inventory_schema` rewrites every
non-scalar property (any field whose type is `array`/`object`, or that carries
`items`/`properties`) into `type: "string"` with a "Pass JSON text in bash
mode" hint, then `make_inventory_callback` runs `coerce_json_text_params` to
parse those JSON strings back into structured values before dispatch. Without
this, bashkit's flag parser falls through to `string` for every non-scalar
type and the dispatcher fails to deserialise generic operations like
`update_agent --capabilities '[{"ref":"current_time"}]'` or
`update_agent --tags '["a","b"]'`. The translation is generic so every
inventory-registered command gets the same treatment automatically; do not
special-case individual fields. JSON parse errors surface a `--<field>:
invalid JSON (...)` message instead of the opaque `callback failed`.

### Structured dispatch errors

`make_inventory_callback` formats every `CommandError` returned from a
generated MCP builtin as `<kind>: <message>`. Bashkit prefixes the builtin
name on its own (`update_model: …`), so the wire output an operator sees is

```
update_model: bad_request: invalid type: string "true", expected bool
```

instead of the historical opaque `update_model: callback failed`. The leading
`<kind>` token is one of the stable lower-snake-case labels emitted by
`command_error_kind`:

| `CommandError` variant | `kind` token   |
| ---------------------- | -------------- |
| `BadRequest`           | `bad_request`  |
| `Unprocessable`        | `unprocessable`|
| `Forbidden`            | `forbidden`    |
| `NotFound`             | `not_found`    |
| `Conflict`             | `conflict`     |
| `Internal`             | `internal`     |

The token set is part of the public MCP contract: do not rename or drop tokens
without a corresponding spec update, and ensure `command_error_kind` covers
every variant when adding a new one (the unit test in `catalog.rs::tests`
enforces this).

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

Domain commands are exposed automatically as bash builtins inside the MCP
scripted tools. The MCP `build_toolset` function in
`crates/server/src/api/mcp_endpoint/catalog.rs` iterates inventory-registered
descriptors:

- `query` filters the command registry to commands whose `read_only()` returns
  true; `execute` exposes the full catalog.
- `CatalogContext::to_domain_ctx()` constructs the domain `Ctx` from the
  MCP `AppState` (db, capability_service, encryption).
- The top-level MCP `discover` tool reads inventory descriptors directly and
  returns JSON with `input_schema`, `output_schema`, and `output_shape` for
  focused searches. This avoids guessing whether a list command should be
  consumed with jq `.data[]` or `.[]`.
- Adding `inventory::submit! { CommandDescriptor::of::<Cmd>() }` makes the
  command immediately discoverable in MCP — no other wiring needed.

Generic runtime dispatch table for non-MCP callers:

```rust
pub async fn dispatch(name: &str, params: Value, ctx: &Ctx) -> Result<String, CommandError> {
    DISPATCH_TABLE.get(name)
        .ok_or_else(|| CommandError::NotFound(format!("Unknown command: {name}")))?
        .dispatch(params, ctx).await
}
```

## gRPC Command Transport

gRPC workers execute inventory-registered management CRUD through the internal
`ExecuteCommand` RPC instead of one bespoke RPC per command. This keeps the
domain command registry as the single entry point across HTTP, MCP, and gRPC.

`ExecuteCommand` contract:

- `name` identifies the inventory command (for example `create_agent`)
- `api_version` is required and currently fixed at `v1`
- `params_json` is a UTF-8 JSON object payload with a 1 MiB limit
- `org_id` scopes execution through `Ctx`
- success returns serialized JSON bytes for the command output
- domain failures return typed command errors (`bad_request`, `forbidden`,
  `not_found`, `conflict`, `internal`) rather than bespoke response structs

Workers can also call `ListCommands` to discover the current command catalog.
Each entry mirrors `CommandMeta`, includes `positional_arg` when present, and
publishes a `schema_hash` derived from command metadata plus positional-arg
shape. That hash is a drift detector for transport consumers; a future
non-backward-compatible command contract change should bump `api_version`.

## Migration Strategy

Domains are migrated incrementally. During migration, both the old service and
the new command can coexist: the service delegates to queries, the command owns
the logic. Once all callers are migrated, the service file is removed.

### Status

All user-facing domains have been migrated. The 30 directories under `crates/server/src/domains/` each own their HTTP/MCP/gRPC surface via commands. Several domains retain an internal `service.rs` alongside `commands.rs` for orchestration that spans multiple commands — that's intentional and not a migration backlog.

**Fully migrated (commands + queries only):** `agents`, `harnesses`, `apps`, `mcp_servers`, `agent_identities`, `skills`, `capabilities`, `audit_logs`, `schedules`, `events`, `images`, `tool_results`, `users`, `organizations`, `system`, `session_databases`, `session_storage`.

**Commands + internal `service.rs`:** `sessions`, `messages`, `budgets`, `evals`, `llm_models`, `llm_providers`, `notifications`, `session_files`, `session_sandbox`, `session_resources`.

**Internal-only (no user-facing commands):** `session_git`, `session_commands`, `session_schedules`.

### Migration order for new domains

1. Create `domains/{name}/` with `mod.rs`, `commands.rs`, `queries.rs`, `types.rs`.
2. Move request types into `types.rs`.
3. Put shared DB helpers in `queries.rs`.
4. Implement `Command` for each user-facing operation in `commands.rs`, each with `inventory::submit! { CommandDescriptor::of::<Cmd>() }`. Declare `policy()` for every non-GET command — the inventory coverage test enforces this.
5. Wire the HTTP adapter to call `cmd.run(&ctx).await` (not `.execute(...)`). MCP and gRPC integration are automatic via inventory.
6. Thread `self.auth.permission_resolver.clone()` into the adapter's `ctx()` method so SaaS custom resolvers apply during enforcement.
7. If orchestration needs to be shared across commands, keep it as a helper under `domains/{name}/` (for example `service.rs`).

### Cross-Org Resource Resolution (`domains/org_resolver.rs`)

`domains/org_resolver.rs` is an inventory-registry module, not a `Command`
domain. Every top-level entity with a dedicated UI detail route MUST
register an `inventory::submit! { ResourceOrgResolver { prefix, resolve } }`
block there so the UI can auto-switch orgs on direct links. See
`specs/multitenancy.md` (Cross-Org Resource Resolution) for the full
contract. When introducing a new such entity, add:

1. A storage method `get_<entity>_organization_id(public_id: &str) -> Result<Option<i64>>`
   (PG + in-memory).
2. One `inventory::submit!` block in `domains/org_resolver.rs`.
3. A UI detail hook that uses `hooks/create-crud-hooks.ts::useDetail`, or a
   bespoke hook that calls `hooks/use-resource-org-fallback.ts` and folds
   `isCheckingOtherOrgs` into its loading state.

### Shared services (`crates/server/src/services/`)

The `services/` layer exists only for code that has no single domain owner.
A file belongs there when **all three** are true:

1. No single domain owns it. If one domain is the natural owner, it moves
   there — even if other domains consume it via free helpers.
2. No user-facing command depends on it as a direct callee. Commands call
   domain code; domain code may call shared services. If an HTTP/MCP/gRPC
   command reaches straight into `services/`, that's a smell.
3. It is one of three shapes: a registry/resolver threaded through `Ctx`,
   an event listener reacting across domains, or a pure algorithm with no
   persistence.

The test: if you can name one domain whose commands would break without the
file and no other, it's not a shared service — move it.

Current shared services (and why each stays):

- `capability` — capability registry held on `Ctx`; used by every domain.
- `event` — event persistence + fanout; called by multiple emitting domains.
- `llm_resolver` — spans `llm_providers`, `llm_models`, and request params;
  no single owner.
- `model_sync` — background listener reconciling provider catalogs with the
  models table.
- `principal` — resolves users and agent identities; shared lookup helper.
- `usage_tracking` — event listener feeding budgets across domains.

Everything else belongs under `domains/<owner>/`. For example, `eval_runner`
lives at `domains/evals/runner.rs`, `scoped_mcp` at
`domains/mcp_servers/scoped_mcp.rs`, `virtual_mount_registry` at
`domains/session_files/virtual_mount_registry.rs`,
`capability_validation` at `domains/capabilities/validation.rs`,
`leased_resource` at `domains/session_resources/leased_resource.rs`.

## Testing

- Commands are tested directly: construct `Ctx` with a test DB, call `execute()`.
- Queries are tested as unit functions: pass a `StorageBackend`, assert results.
- HTTP adapters: integration tests via `axum::test` (unchanged).
- MCP dispatch: test via `dispatch("create_agent", json!({...}), &ctx)`.

## Checklist for adding a new domain

1. Create `domains/{name}/` with `mod.rs`, `commands.rs`, `queries.rs`, `types.rs`
2. Move request types from `api/{name}.rs` to `types.rs`
3. Move business logic from `services/{name}.rs` into `domains/{name}/`
4. Extract shared DB helpers to `queries.rs`
5. Each command: `inventory::submit! { CommandDescriptor::of::<Cmd>() }`
6. Update `api/{name}.rs` to call commands (thin adapter)
7. Remove old top-level service file once all callers migrated
8. Verify `just pre-push` passes
