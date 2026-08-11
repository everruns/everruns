---
type: Specification
title: "Capabilities Specification"
description: "Agent capabilities system."
tags:
  - everruns
  - execution
---
# Capabilities Specification

## Abstract

Capabilities are modular functionality units that extend Agent behavior. A Capability can contribute additions to the system prompt, provide tools, and modify execution behavior. Users can enable multiple capabilities on an Agent to compose functionality.

## Requirements

### Persisted Declarative Capabilities

Declarative capabilities are organization-scoped, database-backed capability
definitions with a stable capability reference shape `declarative:{name}`.
The persisted resource follows the standard dual-ID pattern: an internal UUID
primary key for storage and foreign keys, and a public API ID
`cap_<32-hex>` exposed as `id` in API responses and URLs. They cover capability
contributions that are safe to describe as data rather than custom code:

- System prompt additions
- Scoped remote MCP server configuration
- Text file mounts in the session filesystem
- Skill packages mounted under `/.agents/skills/{name}` and discovered by the
  built-in `skills` capability
- UI feature strings and dependency declarations
- *(Planned)* User-hook bundles (`user_hooks: [UserHookSpec]`) — see
  [user-hooks.md](../runtime-resources/user-hooks.md). The field is **not yet present** on
  `DeclarativeCapabilityDefinition`; when it lands, a non-empty
  `user_hooks` array must force the persisted record's `risk_level` to
  `high` and ride the admin-only assignment gate.

They do not define arbitrary server-side tools. Tool execution still comes from
built-in capabilities, MCP servers, uploaded skills, or client-side tools.
Before runtime assembly, the control plane hydrates the persisted definition
into the selected capability config. This lets dev mode and full worker mode
execute the same declarative capability without workers reading the database.

CRUD API:

- `GET /v1/capabilities` lists built-in, MCP, skill, and active declarative
  capability refs in one registry response.
- `POST /v1/capabilities` creates a persisted declarative capability.
- `GET /v1/capabilities/declarative` lists persisted declarative capability
  resources, including archived records when requested.
- `GET /v1/capabilities/declarative/{public_id}`
- `PATCH /v1/capabilities/declarative/{public_id}`
- `DELETE /v1/capabilities/declarative/{public_id}`
- `POST /v1/capabilities/declarative/{public_id}/delete`

The capability reference used on agents, harnesses, sessions, search results,
and MCP/catalog surfaces is `declarative:<unique_name>`. Names are unique per
organization, lowercase, bounded to fit existing capability reference columns,
and validated before persistence.

Agent and harness write APIs also accept the plain unique name as a convenience:
`{ "ref": "research_pack" }` is normalized to `{ "ref": "declarative:research_pack" }`
before validation and persistence when no built-in capability with that ID
exists. Built-in IDs keep priority to avoid ambiguous references.

Declarative capability resources store both:

- `name`: unique addressable name, used in `declarative:<name>`.
- `display_name`: optional user-facing label. When present, registry/search/UI
  use it as the title while retaining the unique name for references.

Security constraints:

- Records are org-scoped and policy-gated like other agent building blocks.
- Definitions are bounded by server-side size/count limits.
- File mounts are text-only and reject traversal paths.
- Skill names and bundled file paths are validated before persistence.
- MCP server URLs use the same SSRF-safe scoped-MCP validation as harness,
  agent, and session `mcpServers`.
- Declarative dependencies cannot point to other declarative capabilities,
  avoiding persisted dependency cycles across org data.

### Concept

A Capability is an abstraction that defines added functionality for an Agent:

1. **System Prompt Contribution**: Text prepended to the agent's system prompt
2. **Tool Provision**: Tools made available to the agent during execution
3. **Behavior Modification**: Influence on tool invocation and execution (future)

### Agent Handoff Capability

`agent_handoff` is a high-risk orchestration capability for delegating from one
configured Agent to another configured Agent through an allowlist and connection
gate. It is distinct from subagents and blueprints because the target is a
normal Agent resource, not inherited runtime config and not a code-defined
template. See [agent-handoff.md](../runtime-resources/agent-handoff.md).

Both `agent_handoff` and `a2a_agent_delegation` are gated behind the
`agent_delegation` feature flag (experimental, auto-enabled in dev, off in prod).
When the flag is off, neither capability is registered and neither appears in the
capability picker. Enable via `FEATURE_AGENT_DELEGATION=true` or use a
`DeploymentGrade::Dev` environment. See `knowledge/security/feature-flags.md` and EVE-506.

### Guardrail Capabilities

Some capabilities *constrain* agent behavior rather than grant abilities. These
mark themselves with `is_guardrail()` (surfaced as `is_guardrail` on
`CapabilityInfo`) for UI grouping and catalog filtering; the marker has no
runtime semantics. The `guardrails` capability is config-driven: its per-agent
config declares deterministic checks over model output and tool calls that
compile onto the streaming-output and pre/post-tool-hook seams. Guardrails are
opt-in and removable like any capability — there is no org-mandated enforcement
layer. See [guardrails.md](guardrails.md).

### Neutral capability contract

One open identity/configuration contract is shared by the Framework and the
product (EVE-873), owned by `crates/capability` (`everruns-capability`):
validated `CapabilityId`s, the `CapabilityRef` reference/config value,
`CapabilitySpec` plus the non-sealed `IntoCapability` conversion seam, the
code-defined capability authoring surface (`definition` feature), structured
validation/execution errors, and registry identity bookkeeping with
duplicate/collision rejection.

Contract rules:

- `AgentCapabilityConfig` (persisted attachment rows) and
  `BuiltInCapabilityDefinition` (built-in harness provisioning) are the same
  `CapabilityRef` type under historical names — there is no second semantic
  model. The `{"ref", "config"}` wire shape round-trips unchanged through
  Framework activation, persisted attachments, and worker resolution.
- Validation is single-sourced: the Framework's `AgentBuilder::build` and the
  server write paths (`crates/server/src/domains/capabilities/validation.rs`)
  call the same ID grammar and JSON-object-config checks. Reserved namespaces
  (`__everruns_`) and open string IDs are enforced there.
- `CapabilityRef` redacts its config from `Debug`; attachment rows flowing
  through logs never expose config payloads.
- The contract crate depends on neither `everruns-core` nor `everruns-host`
  and carries no Tokio/HTTP/SQLx/OpenAPI/inventory edge, so third-party
  capability crates can depend on it alone
  (`tests/fixtures/external-consumer/capability-pack/` proves this;
  API/DB serialization adapters like the OpenAPI shadow schema in
  `crates/core/src/capability_types.rs` stay thin).
- Architecture guard: `scripts/lib/check-capability-contract.sh` (pre-push
  step and CI job `capability-contract`) fails any new capability
  ID/config/definition type in core, host, or platform.

The effect-neutral `Capability` trait and portable built-ins remain in
`everruns-core`. Environment-backed implementations are owned by focused
sibling crates: filesystem, Bashkit, web fetch, Lua, OpenRouter workspace,
MCP, and concrete HTTP. Hosted product implementations and their service
contracts live in `everruns-platform`.
`scripts/lib/check-environment-capability-isolation.sh` and
`scripts/lib/check-hosted-capability-isolation.sh` guard those dependency
directions and the Framework's opt-in feature edges.

### Architecture

Capability identity/configuration comes from **everruns-capability**.
Effect-neutral built-ins and contracts live in **everruns-core**; environment
implementations live in focused crates; hosted implementations are layered by
**everruns-platform**. All three resolve through the same registry contract
and are composed by the selected host:

```
┌─────────────────────────────────────────────────────────┐
│ everruns-capability + everruns-core                      │
│  ┌─────────────────────────────────────────────────┐   │
│  │ CapabilityRegistry + neutral collection hooks   │   │
│  │ + portable Capability implementations           │   │
│  └─────────────────────────────────────────────────┘   │
└───────────────────────────┬─────────────────────────────┘
                            ↑ platform -> core
┌─────────────────────────────────────────────────────────┐
│                  everruns-platform                      │
│ Hosted knowledge/delegation/task/management impls       │
│ + narrow stores + product registry composition          │
└───────────────────────────┬─────────────────────────────┘
                            ↓
┌─────────────────────────────────────────────────────────┐
│ Focused integrations + host/platform composition         │
│  ┌─────────────────────────────────────────────────┐   │
│  │ filesystem/bashkit/web/lua/MCP/HTTP implementations │
│  │ runtime_capability_registry / hosted registry   │   │
│  └─────────────────────────────────────────────────┘   │
└───────────────────────────┬─────────────────────────────┘
                            ↓
┌─────────────────────────────────────────────────────────┐
│                      Agent Loop                          │
│        (receives fully-configured RuntimeAgent)           │
└─────────────────────────────────────────────────────────┘
```

- Core owns the registry and the portable implementations that need no environment edge or product service.
- `everruns-host::runtime_capability_registry()` adds feature-selected embedded integrations.
- `everruns-platform::capabilities::hosted_capability_registry_for_grade()` adds the hosted product catalog, the hosted integration catalog, and platform-management capabilities.
- The API layer uses the hosted registry and converts it to response DTOs.
- The Agent Loop remains focused on execution
- RuntimeAgent is built with merged system prompt and tools from capabilities

### Live session reconfiguration

The in-process runtime supports changing the session-scoped capability overlay
without rebuilding the runtime or replacing the session:

- `InProcessRuntime::activate_capability(session_id, config)` validates the
  registered capability config, resolves the candidate dependency graph, and
  atomically adds the canonical capability config to the session layer.
- `InProcessRuntime::deactivate_capability(session_id, id)` atomically removes
  a capability previously activated on the session layer.
- Both methods return `CapabilityDelta`. `surfaces_dirty = true` is the
  embedder refresh seam; the following reason/act boundary re-collects the
  system prompt, model-visible tool definitions, executable tools, pre/post
  tool hooks, tool-call hooks, commands, and capability-contributed MCP
  servers from the updated source of truth.
- Repeating an already-satisfied operation succeeds with `changed = false` and
  `surfaces_dirty = false`. Unknown or unavailable capabilities, invalid
  configs, dependency errors, and unsupported session backends fail before any
  runtime surface changes.
- The operation is intentionally session-scoped and non-durable. Embedders own
  persisted enable/disable settings. A session operation cannot remove a
  capability inherited from its harness or agent; that capability must be
  changed at its owning layer.

`SessionMutator::{upsert_session_capability, remove_session_capability}` is the
host-storage seam. Its default implementation fails closed, while the bundled
in-memory runtime store implements both mutations atomically.

### Automatic session titles

The built-in `session` capability may opt into automatic title maintenance.
The default remains manual-only for backward compatibility. When enabled, the
model sets a concise 3–7 word title after the first substantive request and
changes it later only when the conversation's primary theme materially changes,
not for follow-ups or minor subtopics.

Title maintenance is a required pre-work action while automatic titles are
enabled: the initial title and any material primary-theme update happen before
other tools, substantive work, or a substantive response. Session-title writes
change session metadata only and do not count as project or workspace file
changes.

All participating title mutation paths suppress no-ops and emit the typed
`session.title.updated` semantic event after an actual change. Tool-originated
mutations preserve the current turn correlation. Embedders can reuse the core
title-mutation helpers and project events through the runtime event bus without
replacing the capability. See
[`crates/core/src/capabilities/session.rs`](../../crates/core/src/capabilities/session.rs)
for the public config and helper APIs, and [`knowledge/execution/events.md`](events.md) for the
event contract.

#### Deployment Availability (session creation)

Some built-in capabilities are feature-gated (e.g. `container_sandbox` behind
`FEATURE_CONTAINER_SANDBOX`). When a gate is off the capability is absent from
the registry and its tools never register. Session creation therefore rejects
requests whose effective capability set (harness chain + agent + session) names
a **built-in** capability that is not available in this deployment, rather than
silently dropping its tools and degrading into a different execution environment
(e.g. a `coding-container` session quietly running in the bash workspace). The
check runs in `SessionService::create` and only applies to plain built-in
references; namespaced refs (`declarative:`, `plugin:`, `skill:`, `mcp:`) resolve
from org data and are validated separately.

### Data Model

#### Capability (Public DTO)

See `crates/core/src/capability_dto.rs` for the full `CapabilityDto` struct definition.

##### Description Markdown Support

Capability descriptions support GitHub Flavored Markdown including:

- Basic formatting: **bold**, *italic*, `code`, [links](https://example.com)
- Lists (ordered and unordered)
- Code blocks with syntax highlighting
- Tables
- GitHub-style alerts for callouts:

```markdown
> [!NOTE]
> Highlights information that users should take into account.

> [!TIP]
> Optional information to help a user be more successful.

> [!IMPORTANT]
> Crucial information necessary for users to succeed.

> [!WARNING]
> Critical content demanding immediate user attention due to potential risks.

> [!CAUTION]
> Negative potential consequences of an action.
```

Example capability description with alerts:

```
Fetch content from URLs and convert HTML to markdown.

> [!TIP]
> Use `as_markdown: true` for better readability when fetching HTML pages.

> [!WARNING]
> Binary content (images, PDFs) cannot be fetched as text. Only metadata is returned.
```

#### CapabilityId (String wrapper)

Capability IDs are string-based for extensibility. New capabilities can be added without database migrations. See `crates/core/src/capability_types.rs` for `CapabilityId`, `CapabilityStatus`, and `AgentCapabilityConfig` types.

##### ID Format

| Type | Format | Example |
|------|--------|---------|
| Built-in | `snake_case` identifier | `current_time`, `web_fetch` |
| MCP | `mcp:{server_uuid}` | `mcp:01933b5a-0000-7000-8000-000000000501` |
| Declarative | `declarative:{name}` | `declarative:research_pack` |

**Built-in IDs** use `snake_case` naming and are validated against the `CapabilityRegistry`. **MCP IDs** use the `mcp:` prefix followed by the MCP server's UUID. **Declarative IDs** use the `declarative:` prefix followed by the persisted definition's unique name. See `knowledge/integrations/mcp-servers.md` for MCP details.

For the full list of built-in capability IDs, see `crates/core/src/capabilities/mod.rs` (registry initialization).

##### Built-in Capability ID Constants

Every built-in capability **must** declare a `pub const <SCREAMING_SNAKE>_CAPABILITY_ID: &str` in its module and return it from `fn id()`. All constants are re-exported from `crates/core/src/capabilities/mod.rs`.

```rust
// In crates/core/src/capabilities/current_time.rs
pub const CURRENT_TIME_CAPABILITY_ID: &str = "current_time";

impl Capability for CurrentTimeCapability {
    fn id(&self) -> &str {
        CURRENT_TIME_CAPABILITY_ID
    }
    // ...
}
```

Call-sites reference capabilities via the constant rather than a bare string literal. Capabilities whose ID is generated at runtime (MCP, declarative, attach-skill) are exempt — those use prefix constants (`MCP_CAPABILITY_PREFIX`, `DECLARATIVE_CAPABILITY_PREFIX`, `SKILL_CAPABILITY_PREFIX`) and helper constructors instead.

##### ID Aliases

A built-in capability may declare legacy IDs via `Capability::aliases()`. Aliases exist so a capability can be renamed without breaking persisted agent configs: registry lookup (`get`, `has`), dependency resolution, and the high-risk admin gate treat an alias exactly like the canonical ID, and resolution normalizes aliases to the canonical ID (an alias and its canonical ID never activate the capability twice). New configs must use the canonical ID; aliases are a compatibility surface only. Current aliases: `virtual_bash` → `bashkit_shell`.

#### AgentCapabilityConfig (Per-agent configuration)

Associates a capability with an agent via `ref` (CapabilityId) + optional `config` (JSON). The `config` field is capability-specific and passed during execution, allowing per-agent behavior.

**Import compatibility:** For backward compatibility, import accepts both `{ "ref": "id", "config": {} }` and plain `"id"` formats.

#### Capability Trait (everruns-core)

See `crates/core/src/capabilities/mod.rs` for the `Capability` trait and `CapabilityRegistry`. Key trait methods: `id()`, `name()`, `description()`, `status()`, `system_prompt_contribution()`, `facts()`, `tools()`, `mounts()`, `dependencies()`, `features()`, `message_filter_provider()`, `llm_error_hook()`.

##### LLM Error Hook Seam

`llm_error_hook() -> Option<Arc<dyn LlmErrorHook>>` (default `None`) is the
platform seam for **capability-owned error recovery**. It is an *in-process
capability hook* — the same family as `tool_call_hooks()`,
`message_filter_provider()`, and `output_guardrails()` (typed traits invoked
in-process with host services) — as distinct from the user-hook system
(`knowledge/runtime-resources/user-hooks.md`), which runs user-authored shell commands at lifecycle
events. When a turn fails with a *terminal* LLM error (one that will not be
retried), the reason atom collects the hooks contributed by the active
capabilities and invokes each generically — before the user-facing error message
is built — passing an `LlmErrorContext` (session id, classified `error_code`,
structured `error_fields`, the capability's per-agent config, and host
`LlmErrorHookServices`). A hook may perform a side effect and/or return extra
fields to merge into the `UserFacingError` (for example to unlock
capability-specific message copy). The atom stays behavior-agnostic: it knows
nothing about any specific capability's logic, so new error-recovery extensions
are built purely as capabilities. The first consumer is
`usage_limit_auto_continue` (schedules a continuation after a provider usage limit
resets); see `crates/core/src/llm_error_hook.rs`.

##### System Prompt Methods

- **`system_prompt_addition()`** — Sync, returns static `&str`. Used by sync collection path.
- **`system_prompt_contribution(ctx)`** — Async, receives `SystemPromptContext` with session filesystem access. Default wraps `system_prompt_addition()` in XML tags. Capabilities needing dynamic content (e.g., `agent_instructions`, `skills`) override to read from session filesystem.
- **`system_prompt_contribution_with_config(ctx, config)`** — Async, receives per-capability config JSON. Default delegates to `system_prompt_contribution(ctx)`. Used by `collect_capabilities_with_configs()`.

##### Facts (Cache-Friendly Dynamic Context)

`facts(config, ctx) -> Vec<Fact>` lets a capability contribute key/value context to the model without deciding *where* it goes. The runtime routes each `Fact` by its `Volatility` so provider prompt caching is never needlessly invalidated:

| Volatility | Rendered | Cache effect |
|---|---|---|
| `Static` | Folded into a `<facts>` block in the cached system-prompt prefix at build time. | In the cached prefix; assumed stable for the session, so free to cache. |
| `Dynamic` | Appended as a live `<facts>` block at the **conversation tail** on every request (`ReasonAtom`), delivered as a trailing user-role message. | Outside the cached prefix. The system prompt, tools, and conversation history stay byte-identical turn to turn; only the small trailing block is re-processed. |

This is the generic mechanism behind "the current time is X": baking a changing timestamp into the system prompt would bust the system-prompt cache every turn, and a tool round-trip is slower and only informs the model when it asks. `current_time` contributes its value as a `Dynamic` fact (and keeps `get_current_time` for explicit timezone/format queries). Static facts share the same `<facts>` wire format; a single explanatory note (`FACTS_DYNAMIC_NOTE`) is added to the cached prompt once whenever any active capability declares a dynamic fact.

`facts()` is called both at prompt-assembly time (to fold static facts and detect whether any dynamic facts exist) and once per request (to render the live tail block via `collect_dynamic_facts`), so implementations must be cheap and side-effect free. See `crates/core/src/capabilities/facts.rs`.

**Cache-anchor interaction.** Appending a volatile tail would, by default, pull a driver's message-level cache breakpoint onto content that changes every turn — evicting the conversation-history cache. `ReasonAtom` therefore sets `LlmCallConfig.volatile_suffix_len` to the number of trailing volatile messages; the Anthropic driver anchors its breakpoint on the last *stable* block, letting the volatile tail ride as an uncached suffix. Drivers that do not implement message-level anchoring ignore the field and behave exactly as before (`0` is the default).

##### System Prompt Content Contract

System prompt additions must **not duplicate** information already present in tool definitions (names, descriptions, parameter schemas). Only include content that the model cannot infer from tool definitions alone:

- **Behavioral semantics**: when to use which tool, ordering constraints, cross-tool relationships
- **Non-schema constraints**: row limits, naming rules, workspace root paths, scheduling limits
- **Data layout**: filesystem paths where state is persisted (e.g., `/crm/customers.json`)
- **Execution semantics**: what happens when a schedule fires, nesting restrictions

If every piece of information in the prompt is already covered by tool definitions, return `None`. Listing tool names and descriptions in the system prompt is redundant because the model receives tool definitions as structured metadata alongside the prompt.

Prompt contributions are paid on every model call, so capability prompts are
budgeted product surface, not documentation. Authors should keep prompt text to
the smallest stable behavior contract:

- Prefer one or two dense sentences over headings, catalogs, examples, and
  step-by-step tutorials.
- Do not include setup instructions, marketing copy, tool inventories, or
  parameter/action lists when capability metadata, tool descriptions, schemas,
  config schemas, or tool errors already carry that information.
- Do not paste external toolkit `llmtxt`/help output into the runtime prompt
  unless the model needs that grammar on every turn. Long previews may be
  exposed through `system_prompt_preview()` or docs instead.
- Config-aware prompts must describe only behavior that changes with config, and
  must not mention options that are not actually enabled in the current config.
- When a larger prompt is unavoidable (for example, UI component grammars or
  machine-readable output catalogs), document why it cannot live in schemas,
  mounted files, previews, or a tool result.

Prompt-size tests should measure the **actual assembled contribution** using
`system_prompt_contribution(ctx)` or
`system_prompt_contribution_with_config(ctx, config)`, including the
`<capability id="…">` wrapper and config-specific branches. Any capability that
adds or materially changes a runtime prompt should add or update a byte-budget
ratchet test in the same PR. Lowering a budget is always acceptable; raising a
budget requires an explicit code change and a short justification in the commit
or PR description.

##### Config-Aware Methods

- **`tools_with_config(config)`** — Returns tools adapted to per-capability config. Default delegates to `tools()`. Example: `WebFetchCapability` enables `save_to_file` when config has `enable_file_download: true`.

##### Config Schema and UI Metadata

Capabilities that accept per-agent config expose a standard JSON Schema through
`config_schema()` and optional react-jsonschema-form UI hints through
`config_ui_schema()`. These fields are copied to `CapabilityInfo` so clients
render settings through the same generic editor for built-in and integration
capabilities. The backend remains authoritative for config semantics through
`validate_config(config)`, which is called on capability write paths before
config is persisted.

Readability rules for schemas:

- Every exposed property carries an English `title` and `description`.
- Labeled enums use standard `oneOf` `const`/`title` entries (not bare
  `enum`), so clients render human labels without RJSF-specific extensions.
- Only user-meaningful fields are exposed. Advanced or internal tuning fields
  may be consumed by the capability without appearing in the schema, but
  `validate_config` must still tolerate them. Secrets and API keys never go
  through capability config — they flow through user connections and the
  session secret store.

##### Localization

Display strings are localized through additive overlays, keeping
`name()` / `description()` / schema strings as the canonical English base (see
[localization.md](../operations/localization.md) for product-wide locale rules):

- `localizations()` returns `CapabilityLocalization` entries per locale
  (lowercase language tags such as `uk`): localized `name`, `description`,
  a one-line `config_description`, and a `config_overlay` that mirrors the
  JSON Schema structure (`properties`/`items`) with `title`/`description`/
  `enum_labels` leaves. The `en` entry carries only `config_description`.
- `localized_name(locale)` / `localized_description(locale)` /
  `describe_schema(locale)` resolve with the standard fallback chain
  exact tag → language family → `en` → unlocalized trait values.
- The overlay map is copied to `CapabilityInfo.localizations`; clients pick
  the locale and merge the overlay into `config_schema` before rendering, so
  the generic editor stays capability-agnostic. Server-side
  `validate_config` errors remain canonical English; clients localize their
  own pre-submit validation messages.
- Localization affects display only. Prompt building and LLM-facing
  descriptions always use the canonical English strings.

##### Outbound Network Calls

Capabilities that call external HTTP/API endpoints must use `ToolContext`'s
`EgressService` and must pass `ToolContext.network_access` when the destination
URL is authored by the agent, session, capability config, or user input. New
capabilities must not construct direct external HTTP clients in tool execution.
See [egress.md](../operations/egress.md) and [network-access.md](../operations/network-access.md).

##### Collection Functions

- **`collect_capabilities()`** — Sync. Collects tools, mounts, static prompts.
- **`collect_capabilities_async(ctx)`** — Async. Calls `system_prompt_contribution()` for dynamic prompts.
- **`collect_capabilities_with_configs(ctx, configs)`** — Async. Calls config-aware methods (`tools_with_config`, `system_prompt_contribution_with_config`) for capabilities with per-agent config.
- **`RuntimeAgentBuilder::with_capabilities_async(ctx)`** — Async builder method.

**Note**: `message_filter_provider()` returns an optional filter that modifies message retrieval. See [Message Filters](#message-filters).

##### Capability-Contributed Skills

Capabilities may ship reusable skills in code via `contribute_skills() -> Vec<SkillContribution>` (default empty). Each `SkillContribution` carries a name, description, SKILL.md body, bundled files, and invocability flags. During capability collection each contribution is normalized into a read-only mount at `/.agents/skills/{name}/` containing a reconstructed `SKILL.md` plus bundled files. The built-in `skills` capability then discovers and serves them through the same VFS scan used for filesystem and registry skills — no parallel pipeline, no special-case prompt injection, and `/slash` invocability + `disable-model-invocation` flags are honored through the same frontmatter. See `knowledge/project/skills-registry.md` for the discovery/activation contract and `crates/core/src/capabilities/attach_skill.rs` for `SkillContribution` and the underlying mount construction.

### Capability Dependencies

Capabilities can declare dependencies on other capabilities. When a capability is selected, its dependencies are automatically resolved and applied.

#### How Dependencies Work

1. **Declaration**: Capabilities implement `dependencies()` returning a list of capability IDs they depend on
2. **Resolution**: When capabilities are applied, dependencies are resolved recursively
3. **Ordering**: Dependencies are always applied before dependents (topological order)
4. **Storage**: Only user-selected capabilities are stored in the database; dependencies are resolved at runtime
5. **UI Enforcement**: Users cannot remove a capability that is required by other selected capabilities

#### Dependency Resolution Rules

- Dependencies are resolved depth-first
- Circular dependencies are detected and return an error
- Maximum of 100 resolved capabilities to prevent resource exhaustion
- Unknown capabilities in dependency chains are silently skipped

#### Example

When a capability that depends on `session_file_system` is selected (e.g. the
`sample_data` fixture from `everruns-test-support`):
1. `session_file_system` is added as a dependency (if not already selected)
2. `session_file_system` tools and system prompt are applied first
3. The dependent capability's mounts and system prompt are applied second
4. User cannot remove `session_file_system` while the dependent is selected

#### Dependency Resolution API

See `crates/core/src/capabilities/mod.rs` for `resolve_dependencies()` and `ResolvedCapabilities`.

#### Built-in Capabilities with Dependencies

| Capability | Depends On | Features |
|------------|-----------|----------|
| `skills` | `session_file_system` | *(none)* |
| `bashkit_shell` | `session_file_system` | `file_system` |
| `gpt_image_gen` | `session_file_system`, `session_storage` | *(none)* |

#### Tool-Side Provider Clients

Some capabilities call provider APIs directly instead of routing through the LLM driver abstraction. For those cases the worker-side `ToolContext` may include:

- `provider_credential_store` for resolving default provider credentials without reading provider environment variables in tool code
- `image_store` for durable image artifact persistence and later reuse by ID

`gpt_image_gen` is the first built-in example of this pattern: it resolves OpenAI credentials through session secrets or the provider credential store, persists generated outputs in org-scoped image storage, and can optionally mirror them into the session filesystem.

### Capability Features

Capabilities declare UI features they contribute to via `features()`. Features are open-ended strings indicating what user-facing functionality a capability enables. The Session DTO aggregates features from all active capabilities (harness + agent + session-level) at read time, and the UI uses the aggregated set to decide which tabs/sections to render.

#### How Features Work

1. **Declaration**: Capabilities implement `features()` returning a list of feature strings
2. **Aggregation**: `compute_features()` resolves dependencies and collects features from all resolved capabilities
3. **Deduplication**: Features are deduplicated — multiple capabilities contributing the same feature produce one entry
4. **Session DTO**: The `Session.features` field is computed at read time (not stored in the database)
5. **UI Rendering**: The UI conditionally renders tabs based on the feature set (e.g., Workspace tab only appears when `"file_system"` is present)

#### Built-in Feature Mapping

| Capability | Features |
|------------|----------|
| `session_file_system` | `file_system` |
| `bashkit_shell` | `file_system` |
| `session_storage` | `secrets`, `key_value` |
| `session_schedule` | `schedules` |
| `session_sandbox` | `managed_sandbox` |
| `session_sql_database` | `sql_database` |

| `openui` | `openui` |
| `a2ui` | `a2ui` |

MCP and Skill virtual capabilities currently declare no features.

#### Known Feature Strings

| Feature | UI Element | Description |
|---------|------------|-------------|
| `file_system` | Workspace tab | Session has file system access |
| `secrets` | Storage tab | Session can store encrypted secrets |
| `managed_sandbox` | Sandbox-aware UX | Session has one managed sandbox with provider-neutral tools |
| `key_value` | Storage tab | Session can store key/value pairs |
| `schedules` | Schedules tab | Session can create/manage schedules |
| `sql_database` | *(reserved)* | Session has SQL database access |
| `openui` | OpenUI rendering | Session can render OpenUI Lang components |
| `a2ui` | A2UI rendering | Session can render A2UI JSON component trees |

#### compute_features()

See `crates/core/src/capabilities/mod.rs` for `compute_features()` — resolves dependencies, collects features, deduplicates.

### Risk Levels (TM-AGENT-005)

Each capability declares a `RiskLevel` via the `Capability` trait. The API enforces that assigning high-risk capabilities to an agent requires `OrgRole::Admin`.

| Level | Description | Enforcement |
|-------|-------------|-------------|
| `low` | Default. No special requirements. | Any org member |
| `medium` | Logged but allowed for org members. | Any org member |
| `high` | Can access external compute or resources, or perform unrestricted outbound network calls, or run scripted code execution. | Requires Admin role |

High-risk built-in capabilities: `docker_container`, `daytona`, `e2b`, `deno`, `bashkit_shell`, `web_fetch`.

See `crates/core/src/capabilities/mod.rs` for the `RiskLevel` enum and `crates/server/src/api/agents.rs` for `require_admin_for_high_risk()`.

#### Admin-Only Tier Decision (PRs #1485, #1500, EVE-395)

`bashkit_shell` (previously `Low`) and `web_fetch` (previously `Medium`) were elevated to `High` and the high-risk admin gate now applies to them.

- **Trust rationale.**
  - `bashkit_shell` is sandboxed (workspace-only FS, no real network), but exposes scripted code execution. Combined with LLM-driven invocation it is a meaningful trust elevation versus single-purpose tools.
  - `web_fetch` doubles as a data-exfiltration channel (TM-AGENT-013) and a partial SSRF vector. Loopback/RFC1918 are blocked with DNS pinning (TM-API-008), and outbound URL filtering now exists via per-layer `NetworkAccessList` plus the system allowlist at the egress boundary (TM-AGENT-018 MITIGATED, `knowledge/operations/network-access.md`) — but both default to open, so admin assignment remains the explicit trust gate for enabling the capability.
- **Gate location.** Canonical agent create / update enforcement lives in `check_high_risk_caps` (`crates/server/src/domains/agents/commands.rs`), invoked from `CreateAgent::execute`, `UpdateAgent::execute`, and `UpsertAgent::execute`. The sibling `require_admin_for_high_risk` helper in `crates/server/src/api/agents.rs` enforces the same contract on agent-import / copy paths. Member-attempted assignments are rejected with HTTP 403 and an error message that names the offending capability ids.
- **Migration / grandfathering.** The gate is *not* enforced at runtime. Member-owned agents that already had `bashkit_shell` or `web_fetch` before this change continue to run. Members can still edit other fields on those agents; the gate fires only when a member request would *add or keep* a high-risk capability that violates the role contract on a write that is itself reshaping the capability set.
- **Member experience.** Members trying to add `bashkit_shell` or `web_fetch` see a 403 with the locked capability id. Admin approval is the path: an admin must either own the agent or perform the update on the member's behalf. A self-serve "request admin approval" UI flow is not part of this contract; it is tracked separately and may be added later without breaking the gate.
- **What this is *not*.** This is not a per-capability policy override. There is no env var or org setting today that downgrades `bashkit_shell` / `web_fetch` to member-assignable. If product later wants a tunable, it will be added explicitly (org-level policy `members_can_use_high_risk_capabilities` per-capability override) rather than emerging from constant edits in capability source files.

### Built-in Capabilities

For the full list of built-in capabilities with their tools, parameters, and system prompts, see `crates/core/src/capabilities/` (each capability has its own module).

#### FileSystem

- **ID**: `session_file_system`
- **Purpose**: Tools to access and manipulate files in the session workspace (`/workspace`)
- **Tools**: `read_file`, `write_file`, `edit_file`, `list_directory`, `grep_files`, `delete_file`, `stat_file`
- **Workspace Mount**: All paths relative to `/workspace`, normalized internally for bashkit_shell integration

##### Design Decision: Context-Aware Tools

FileSystem tools require session context to access the isolated virtual filesystem. Each tool implements `requires_context() -> true` and uses `execute_with_context()` instead of the standard `execute()`. The `ToolContext` provides:
- `session_id`: The session whose filesystem to access
- `file_store`: A `SessionFileSystem` trait implementation for file operations

See `knowledge/runtime-resources/file-store.md` for the pluggable `SessionFileSystem` contract and
the in-memory and real-disk implementations. New capabilities that need
filesystem access should go through `ToolContext.file_store` or
`SystemPromptContext.file_store` rather than calling `std::fs` directly,
except for host-process tools (e.g. the bash tool) where the shell
inherits the host filesystem regardless of which `SessionFileSystem` is plugged
in.
The active session filesystem is selected by the platform's
`SessionFileSystemFactory` unless an embedder supplies an explicit runtime
backend override.

##### Design Decision: Session Isolation

Each session has its own isolated filesystem stored in PostgreSQL. Files are session-scoped and cannot be accessed across sessions. This provides security isolation and clean separation between conversations.

##### Design Decision: Auto-Create Parents

When writing a file like `/a/b/c.txt`, parent directories `/a` and `/a/b` are automatically created if they don't exist. This follows common filesystem patterns and reduces tool call overhead.

##### Design Decision: Hash-Gated Text Edits

`edit_file` is intentionally narrower than `write_file`: it only edits existing text files, requires a `content_hash` from `read_file` or `write_file`, and applies one or more exact replacements atomically. If the hash is stale, all replacements are preflighted as unique and non-overlapping against the current content, allowing an unrelated concurrent change to survive; a missing, ambiguous, overlapping, or racing hunk fails without modifying the file. The final write uses compare-and-set semantics so later concurrent writes are never clobbered. This keeps the tool model-friendly while preventing conflicted writes and accidental binary corruption.

#### BashkitShell

- **ID**: `bashkit_shell` (legacy alias: `virtual_bash`)
- **Purpose**: Sandboxed bash shell execution for safe code execution and file manipulation
- **Dependencies**: `session_file_system` (operates on same workspace)
- **Tools**: `bash` (execute command with optional working_dir and timeout)
- **Workspace Mount**: Session files at `/workspace`, shared with FileSystem tools

##### Naming Convention: `*_shell` Family

Shell-execution capability IDs name the engine plus the `_shell` suffix: `bashkit_shell` today; a future native or remote engine would be e.g. `native_shell` or `aws_shell`, and an engine-selecting wrapper would be `auto_shell`. The tool name `bash` is intentionally shared across the family so agents and harnesses stay portable across engines.

##### Design Decision: bashkit Library

This capability uses the [bashkit](https://github.com/everruns/bashkit) library for sandboxed bash execution. bashkit provides:
- WASM-like execution isolation (no system access)
- Custom FileSystem trait implementation for session file access
- Built-in shell commands (cd, ls, cat, echo, etc.)
- Resource limits to prevent infinite loops

##### Design Decision: SessionFileSystemAdapter

The `SessionFileSystemAdapter` implements bashkit's `FileSystem` trait, bridging bash file operations to the session filesystem. This enables:
- **Live file visibility**: Files written by other tools during bash execution are immediately visible
- **No sync overhead**: Eliminates pre/post execution sync of entire filesystem
- **Memory efficiency**: Files read on-demand instead of loading all into memory
- **Consistency**: Single source of truth for file state

##### Design Decision: Path Translation

Bash operates with `/workspace` as the root. The adapter translates paths:
- `/workspace/foo.txt` → `/foo.txt` (storage path)
- Paths outside `/workspace` return errors

This ensures bash and FileSystem capability share the same file namespace.

#### SessionStorage

- **ID**: `session_storage`
- **Purpose**: Session-scoped key/value storage and encrypted secret storage
- **Tools**: `kv_store` (set/get/delete/list), `secret_store` (encrypted, same ops)

##### Design Decision: Consolidated Tools

Two tools consolidate 8 operations (4 per storage type) to reduce tool count:
- `kv_store` with `operation` parameter (set/get/delete/list) for key/value storage
- `secret_store` with `operation` parameter (set/get/delete/list) for encrypted secrets

##### Design Decision: Key/Value vs Secrets

Two storage types are provided:
- **Key/Value storage**: For general data like state, preferences, or intermediate results. Stored as plain text in the database.
- **Secret storage**: For sensitive data like API keys, tokens, or credentials. Values are encrypted at rest using AES-256-GCM envelope encryption.

##### Design Decision: Session Isolation

Like the FileSystem capability, storage is session-scoped. Keys and secrets cannot be accessed across sessions.

##### Design Decision: Upsert Semantics

Both set operations use upsert semantics - storing with an existing key/name will overwrite the previous value.

##### Design Decision: Encryption Requirement for Secrets

Secret operations require the `SECRETS_ENCRYPTION_KEY` environment variable to be configured. If encryption is not configured, secret operations will return an error. Key/value operations work without encryption.

#### WebFetch

- **ID**: `web_fetch`
- **Purpose**: Fetch content from URLs and convert HTML to markdown or plain text
- **Tools**: `web_fetch` (url, method, as_markdown, as_text)
- **Timeouts**: First byte 1s, body 30s (partial content on timeout)

##### Design Decision: System Prompt from fetchkit

This capability uses the `TOOL_LLMTXT` constant from fetchkit as its system prompt addition. This provides comprehensive LLM-optimized documentation including:
- Capability overview
- Input parameters with descriptions
- Output fields with explanations
- Usage examples
- Error handling guidance

##### Design Decision: Binary Content Metadata

Binary content (images, PDFs, audio, video, etc.) cannot be fetched as textual content. However, instead of returning a tool error, the response includes available metadata (content type, size, filename, last modified) along with an error message explaining binary content is not supported. This allows agents to understand what the resource is even though they cannot access its content.

##### Design Decision: fetchkit Library

The capability uses the [fetchkit](https://github.com/everruns/fetchkit) library (v0.1.2) for all HTTP operations and HTML conversion. This provides:
- Consistent behavior across deployments
- Built-in HTML to markdown/text conversion optimized for LLM consumption
- Automatic binary content detection
- Timeout management with partial content on body timeout
- Excessive newline filtering
- **SSRF protection** via `DnsPolicy::block_private_ips()` (default): resolve-then-check validates resolved IPs against blocked ranges (loopback, RFC1918, link-local/cloud metadata, CGNAT, multicast) with DNS pinning to prevent rebinding attacks

#### StatelessTodoList

- **ID**: `stateless_todo_list`
- **Purpose**: Structured task lists for tracking multi-step work progress
- **Tools**: `write_todos` (replaces entire todo list each call; state in conversation history)

##### Design Decision: Stateless Implementation

This capability is intentionally **stateless** - it does not persist todos to a separate database table. State is maintained through conversation history (message storage).

###### Why Stateless?

This follows the same pattern as Claude Code's TodoWrite tool:
- Each `write_todos` call receives and returns the **complete** todo list
- The LLM remembers todos by reading previous tool calls from conversation history
- No separate storage layer needed - simpler implementation

###### Alternative Approaches (Research)

**LangChain DeepAgents TodoListMiddleware**:
- Uses dedicated `todos` state channel (not message history)
- Thread-scoped lifecycle with subagent isolation
- Known issue: context tokens grow quickly (proposed `auto_clean_context` flag)
- Reference: https://deepwiki.com/langchain-ai/deepagents/2.4-state-management

**OpenAI Codex CLI update_plan**:
- Tool named `update_plan` with explanation + plan items
- Maintains plan history across resumed runs
- Supports "compacting conversation state" for longer sessions
- Reference: https://github.com/openai/codex

###### Trade-offs

| Approach | Pros | Cons |
|----------|------|------|
| Stateless (current) | Simple, no DB changes | Context grows with messages |
| State channel | Efficient context | Complex middleware needed |
| DB persistence | Survives context loss | Requires schema changes |

###### Future Improvements

Consider adding context compaction (prune old `write_todos` calls) if context growth becomes an issue in long-running sessions.

##### When to Use StatelessTodoList

The system prompt instructs agents to use task management when:
1. **Complex multi-step tasks** - Tasks requiring 3 or more distinct steps
2. **User provides multiple tasks** - When users give a list of things to do
3. **Non-trivial work** - Tasks requiring careful planning
4. **After receiving new instructions** - Capture requirements immediately
5. **When starting work** - Mark a task as `in_progress` BEFORE beginning
6. **After completing work** - Mark task as `completed` and add follow-up tasks

##### Best Practices

1. **One task in progress** - Exactly one task should be `in_progress` at a time
2. **Update immediately** - Mark tasks completed as soon as done, don't batch
3. **Replace entire list** - Each `write_todos` call replaces the full list
4. **Completion criteria** - Only mark `completed` when fully done (tests pass, no errors)

#### OpenUI

- **ID**: `openui`
- **Purpose**: Enables agents to generate rich interactive UI (charts, tables, forms, cards) using OpenUI Lang
- **Tools**: None (prompt-only capability)
- **Features**: `openui`
- **Icon**: `layout`
- **Category**: `UI`

##### Design Decision: Prompt-Only Capability

OpenUI is a pure system prompt capability — it provides no tools. It instructs the LLM to output OpenUI Lang code in ` ```openui ` fenced code blocks. The UI frontend detects and renders these blocks using `@openuidev/react-lang` and `@openuidev/react-ui`.

##### Design Decision: Component Library from Crate

The system prompt is generated from the `everruns-openui` crate which defines the full component library (50+ components across 7 groups: Content, Tables, Charts 2D, Charts 1D, Forms, Buttons, Layout). Component signatures are included in the prompt so the LLM knows valid props.

See `knowledge/ui/openui.md` for full architecture and UI integration details.

#### A2UI

- **ID**: `a2ui`
- **Purpose**: Enables agents to emit Google A2UI JSON component trees, rendered by the UI with native shadcn/ui primitives
- **Tools**: None (prompt-only capability)
- **Features**: `a2ui`
- **Icon**: `layout-grid`
- **Category**: `UI`

##### Design Decision: Parallel to OpenUI

A2UI is a *parallel* capability to `openui`, not a replacement. Both are opt-in per agent; neither is enabled by default. A2UI uses JSON (portable across frameworks) while OpenUI uses a custom DSL with a richer prebuilt React library. Enabling both is legal but usually wasteful.

##### Design Decision: Prompt-First, Native Renderer

Follows the A2UI v0.9 prompt-first model: the catalog is embedded in the system prompt and the LLM emits JSON freely, validated lightly at render time. The UI renderer is implemented using the project's own shadcn/ui primitives — no third-party npm dependency. This preserves design-system consistency and is A2UI's canonical pattern (the client owns the rendering).

See `knowledge/ui/a2ui.md` for full architecture, the canonical catalog, and the wire format.

#### SkillsDiscovery

- **ID**: `skills`
- **Purpose**: Discover and activate skills from `/.agents/skills/` in session VFS
- **Dependencies**: `session_file_system`
- **Tools**: `list_skills` (scan for SKILL.md files), `activate_skill` (load instructions by name)

##### Design Decision: VFS-Based Discovery

This is the built-in skills discovery mechanism. It does NOT ship with any skills — users upload SKILL.md files to `/.agents/skills/{name}/SKILL.md` in the session filesystem. The agent discovers them at runtime via `list_skills` and activates them on demand via `activate_skill`.

This complements registry-based skills (`skill:{uuid}` capabilities) which are database-stored and org-wide. VFS-discovered skills are per-session and project-specific.

##### Design Decision: Progressive Disclosure

Following the agentskills.io specification:
1. **Discovery**: `list_skills` returns only names and descriptions (~100 tokens per skill)
2. **Activation**: `activate_skill` loads full SKILL.md instructions (<5000 tokens recommended)
3. **Resources**: Bundled files listed in activation response, accessible via existing session filesystem tools

#### LoopDetection

- **ID**: `loop_detection`
- **Purpose**: Detects repeated tool loops and injects a warning to break the loop
- **Tools**: None (uses `MessageFilterProvider::post_load` only)
- **Config**: `{"threshold": N}` — number of repeated identical results, identical tool-call batches, or read ranges before warning (default 3)
- **Source**: `crates/core/src/capabilities/loop_detection.rs`

#### ToolApproval

- **ID**: `tool_approval`
- **Purpose**: Suspends the turn and asks a human before a risky tool runs
- **Status**: Not registered by default — it needs a host that can service an interactive prompt. Hosts construct `ToolApprovalCapability::new(approver)` and register it through their `PlatformDefinition`.
- **Tools**: None (contributes a `PreToolUseHook`)
- **Config**: `{"mode": "off" | "normal" | "protective"}` (default `normal`)
- **Source**: `crates/core/src/capabilities/tool_approval.rs`
- **Behavior**: Classifies each call by the risk the tool *declares* through `ToolHints` — `readonly` is never gated, `destructive`/`open_world` always can be, and an un-annotated tool fails safe as mutating. `normal` asks before destructive/outward calls; `protective` asks before anything that is not read-only; `off` never asks. "Always" answers are remembered per (session, tool). A host that cannot be reached answers `Unavailable`, which allows the call — a client with no permission UI keeps working autonomously instead of deadlocking on every mutating tool. Ported from yolop, where the ACP server backs the approver with the client's `session/request_permission`.

#### ProgressGuard

- **ID**: `progress_guard`
- **Purpose**: Interrupts investigation that is not producing progress — many exploration tools without an edit or a validation run, repeated identical searches, zero-evidence searches, re-reading unchanged files, re-running a validation against an unchanged workspace, and poll-loops waiting on external events
- **Status**: Registered, opt-in per agent. Behavior-only.
- **Tools**: None (contributes a `PostToolExecHook`)
- **Config**: None
- **Source**: `crates/core/src/capabilities/progress_guard.rs`
- **Behavior**: Observes tool traffic per session and appends a warning to the *next* tool result when a threshold trips, naming the situation and the required next action. Runtime-enforced rather than prompt-only, and contributes no system prompt text: a warning that usually never fires should not be paid for on every turn. Complements `loop_detection`, which catches literal repeats of the same call; this catches activity that varies but goes nowhere. Ported from yolop.

#### ToolCallRepair

- **ID**: `tool_call_repair`
- **Purpose**: Detects and repairs malformed tool-call arguments from the model, recovering the turn instead of surfacing a raw parse error
- **Status**: Opt-in. Registered but **disabled by default** — contributes nothing unless an agent explicitly enables it; with it off, behavior is byte-for-byte unchanged.
- **Tools**: None (intercepts in the `reason` atom, after tool calls are finalized and before the assistant message is built)
- **Config**: `{"max_reprompts": N}` — corrective re-prompt attempts allowed per malformed call before falling through to the existing error path (default 1, range 0–5)
- **Source**: `crates/core/src/capabilities/tool_call_repair.rs`
- **Behavior**: Runs a pure, table-tested `salvage_tool_arguments` over each call's `arguments`: unwraps fenced ```json blocks and surrounding prose, strips trailing commas, normalizes single quotes, and coerces string-typed known keys against the tool's JSON schema (`"42"` → `42`). The already-valid case is a no-op. When local salvage fails the bounded re-prompt path applies (capped by `max_reprompts`), then falls through to today's exact error. Emits one `tool.call_repaired` event per malformed call with an outcome label (`local-salvage` | `re-prompt` | `gave-up`). Salvage parses untrusted model output and is bounded: inputs over 256 KiB are rejected and all scanning is linear and non-recursive.

#### MessageMetadata

- **ID**: `message_metadata`
- **Purpose**: Annotates user and agent messages with metadata (message timestamp, UTC) in the prompt-facing model view so the model can reason about timing and gaps between messages
- **Tools**: None (uses `ModelViewProvider` only; priority 100, after compaction masking)
- **Config**: `{"fields": ["timestamp"]}` — which metadata fields to render, in order (default `["timestamp"]`; `[]` disables annotations). User and agent messages are always annotated; system and tool-result messages never are.
- **Source**: `crates/core/src/capabilities/message_metadata.rs`
- **Behavior**: Prefixes the first text part of each user/agent message with one bracketed segment per configured field (e.g. `[time <RFC3339 UTC>]` from `Message::created_at`); tool-call-only agent messages get the annotation as a leading text part. Fields are an extensible enum (`MessageMetadataField`); future fields (e.g. the LLM model behind an agent message, once stored messages record it) render their own segment and may skip messages that lack the data. Stored messages are unchanged, and `created_at` is immutable, so annotations are deterministic across turns and do not invalidate provider prompt caches. A small system prompt addition explains the annotation and forbids the model from emitting it.

#### PromptCanaryGuardrail

- **ID**: `prompt_canary_guardrail`
- **Purpose**: Streaming output guardrail that withholds the assistant message when the model echoes the first sentence of its system prompt
- **Tools**: None (uses `output_guardrails()` only)
- **Config**: `{"replacement": "..."}` — optional replacement text shown in place of the leak (default: a generic "withheld" message)
- **Risk**: `Low` — read-only inspection of model output
- **Source**: `crates/core/src/capabilities/prompt_canary_guardrail.rs`
- **Behavior**: At stream arm time, extracts the first sentence of the assembled system prompt whose normalized form is ≥ 30 chars and uses it as a substring needle (lowercased, whitespace-collapsed). Each batched delta runs the substring check; on hit, the stream is aborted and `output.message.replaced` is emitted. The original tokens are never persisted or replayed. See [Output Guardrails](#output-guardrails).

### MCP Virtual Capabilities

MCP servers (see `knowledge/integrations/mcp-servers.md`) are integrated as "virtual capabilities" alongside built-in capabilities. This allows agents to use MCP tools through the same capability selection UI.

#### Capability ID Format

MCP capabilities use a prefixed ID format:
```
mcp:{server_uuid}
```

Example: `mcp:01933b5a-0000-7000-8000-000000000501`

The UUID portion is the MCP server's unique identifier (UUID v7).

#### How MCP Capabilities Work

1. **Discovery**: When listing capabilities, active MCP servers are included with their cached tools
2. **Selection**: Agents can select MCP capabilities just like built-in capabilities
3. **Tool Execution**: MCP tools are prefixed with `mcp_{server_name}__` (double underscore) to avoid conflicts
4. **UI Badge**: MCP capabilities display an "MCP" badge in the capability selector

#### MCP Tool Name Format

MCP tools use a double-underscore separator to distinguish server name from tool name:

```
mcp_{server_name}__{tool_name}
```

**Why double underscore?** Server names can contain underscores (e.g., `microsoft_learn`), so a single underscore would be ambiguous. The double underscore provides unambiguous parsing.

**Examples:**
| Server | Tool | Full Tool Name |
|--------|------|----------------|
| `github` | `search_repos` | `mcp_github__search_repos` |
| `microsoft_learn` | `search` | `mcp_microsoft_learn__search` |
| `atlassian_jira` | `create_issue` | `mcp_atlassian_jira__create_issue` |

#### Key Differences from Built-in Capabilities

| Aspect | Built-in | MCP |
|--------|----------|-----|
| Source | Rust code | Remote server |
| ID format | `snake_case` | `mcp:{uuid}` |
| Tool prefix | None | `mcp_{server}__` |
| Tool discovery | Compile-time | Runtime (cached, 24h TTL) |
| Execution | In-process | HTTP JSON-RPC |
| Availability | Always | Depends on server status |
| System prompt | Optional per-capability | None |
| Dependencies | Supported | Not supported |

See `knowledge/integrations/mcp-servers.md` for full MCP integration details including:
- Server configuration and authentication
- Tool discovery protocol
- Tool execution flow
- Error handling

### Capability Application Flow

When a session executes:

1. **Load Agent**: Fetch agent configuration from database
2. **Load Session**: Fetch session configuration including session-level capabilities
3. **Fetch Agent Capabilities**: Get agent's enabled capabilities via `get_agent_capabilities(agent_id)`
4. **Resolve Capabilities**: For each capability (ordered by `position`):
   - Look up `InternalCapability` from registry by string ID
   - Call `system_prompt_contribution(ctx)` (async) for dynamic prompt content
   - Collect `tools` definitions
5. **Apply Session Capabilities**: Session-level capabilities are applied **after** agent capabilities (additive):
   - Session capabilities extend the agent's tools
   - Session capability system prompts are prepended after agent capability prompts
   - This allows temporarily extending capabilities without modifying the agent
6. **Build RuntimeAgent**:
   - System prompt = session cap additions + agent cap additions + agent's base prompt
   - Each section is wrapped in XML tags for clear boundaries (see `knowledge/project/xml-prompt-formatting.md`):
     - Instruction file content: `<agent-instructions source="{path}">`
     - Each capability's addition: `<capability id="{cap_id}">`
     - Base prompt: `<system-prompt>` (only when capabilities contribute prompt additions)
   - Tools = merged tool list from agent capabilities + session capabilities
7. **Execute**: Run Agent Loop with fully configured RuntimeAgent

### Session Capabilities

Sessions can have their own capabilities that are additive to the agent's capabilities. This enables:
- Temporarily extending an agent's capabilities for specific sessions
- Per-session customization without modifying the agent configuration
- Testing new capabilities before adding them to the agent

Session capabilities use the same format as agent capabilities:

```json
{
  "capabilities": [
    { "ref": "current_time" },
    { "ref": "web_fetch", "config": { "timeout_ms": 30000 } }
  ]
}
```

Session capabilities are stored in the `sessions.capabilities` column (JSONB) and applied at runtime via `RuntimeAgentBuilder::with_capabilities()` after agent capabilities are applied.

### API Endpoints

Capabilities are managed as part of the agent resource. When creating or updating an agent, you can specify the capabilities to enable. The agent response includes the list of enabled capabilities.

All endpoints are organization-scoped.

| Method | Path | Description |
|--------|------|-------------|
| GET | `/v1/capabilities` | List all available capabilities |
| GET | `/v1/capabilities/{capability_id}` | Get capability details |

Agent capabilities are managed through the agents API:
- `POST /v1/agents` - Create agent with capabilities
- `PATCH /v1/agents/{id}` - Update agent capabilities
- `GET /v1/agents/{id}` - Get agent (includes capabilities)

Capabilities are managed via the agents API (`POST /v1/agents`, `PATCH /v1/agents/{id}`). The capabilities array order determines priority (earlier = first in system prompt). See `knowledge/execution/apis.md` and the OpenAPI spec for request/response formats.

### Database Schema

See `crates/server/migrations/001_base_schema.sql` for the `agent_capabilities` table DDL.

**Note**: The `capability_id` column has no CHECK constraint — validation is at the application layer via `CapabilityRegistry`. This allows adding capabilities without database migrations.

### Design Decisions

| Question | Decision |
|----------|----------|
| Where are capabilities defined? | In-memory registry (not database) |
| How are they applied? | Resolved at API layer, merged into RuntimeAgent |
| Order of application? | By `position` field (lower = earlier) |
| Can capabilities conflict? | Currently no conflict resolution; later capabilities add to earlier ones |
| Can users create custom capabilities? | Not in current version (built-in only) |
| How are new capabilities added? | Implement `Capability` trait with unique `id()` and register in `CapabilityRegistry` - no database changes or central ID definitions needed |

### Adding New Capabilities

1. Implement the `Capability` trait (see `crates/core/src/capabilities/mod.rs` for the neutral trait definition) in the crate that owns the implementation's effects.
2. Declare `pub const <SCREAMING_SNAKE>_CAPABILITY_ID: &str = "<id>";` in that owner and return it from `fn id()` (see **Built-in Capability ID Constants** above).
3. Re-export the constant and struct from the owning crate's public root: portable implementations from `everruns-core`, environment-backed ones from their integration crate, hosted/service-backed ones from `everruns-platform`.
4. Choose the registry preset deliberately:
   - Register effect-neutral kernel capabilities in `CapabilityRegistry::runtime_builtins()` and/or `CapabilityRegistry::with_builtins_for_grade()`.
   - Register environment-backed embedded capabilities in `everruns_host::runtime_capability_registry()` behind a Cargo feature.
   - Register hosted/service-backed capabilities through `everruns_platform::capabilities::register_hosted_capabilities()`, which `hosted_capability_registry_for_grade()` composes with the product feature set.
   - Use integration inventory registration for external integration crates that should appear only when their crate is linked.
5. Add tool implementations if needed (implement `Tool` trait from `crates/core/src/tools.rs`)
6. No database migration required — capability ID validated at runtime
7. Add or update registry tests that prove the capability appears in the intended preset(s) and is absent from inappropriate preset(s)
8. Update documentation at `docs/capabilities/` — add a page for the new capability and update the overview table in `docs/capabilities/index.md`

Runtime-default eligibility is based on host services, not risk level. A high-risk
capability may be runtime-usable when it runs entirely through runtime-provided
services such as the session filesystem or egress service. A low-risk capability
must still stay out of `runtime_builtins()` if its tools need optional services
such as platform, task, schedule, provider-credential, or knowledge stores. See `knowledge/foundations/runtime.md` for
the embedded runtime contract.

### Capability Mount Points

Capabilities can declare mount points to populate files and directories in the session filesystem when a session is created. This allows capabilities to provide sample data, documentation, configuration files, or other resources that agents can access during execution.

#### Mount Point Data Model

See `crates/core/src/capability_types.rs` for `MountPoint`, `MountAccess`, `MountSource`, and `MountEntry` types.

Key concepts:
- `MountAccess`: `ReadOnly` or `ReadWrite`
- `MountSource`: `InlineFile` (content + encoding), `InlineDirectory` (HashMap of entries), or `Virtual` (in-memory `VirtualFileTree` via `Arc`)
- Capabilities declare mounts via the `mounts()` trait method

#### Virtual Mounts

Virtual mounts (`MountSource::Virtual`) serve content from an in-memory `VirtualFileTree` without writing rows to the `session_files` table. Content is typically embedded at compile time via `include_dir!` and shared across sessions via `Arc`. Virtual mounts are always read-only.

- **Registration**: During `apply_capability_mounts()`, virtual mounts are registered in the `VirtualMountRegistry` (per-session, in-memory) instead of being written to the database.
- **Read path**: `WorkspaceFileService`, `DirectWorkerAdapters`, and the bashkit shell all check the virtual registry before querying the database. Virtual files win on path conflicts.
- **Merge semantics**: `list_directory` merges virtual and DB entries (virtual wins on name conflict, sorted dirs-first then by path). `grep` searches both sources and concatenates results.
- **Write protection**: Writes/deletes to virtual paths return a readonly error.
- **Eviction**: Virtual mount entries for a session are evicted from the registry on session delete.
- **Lazy reconstruction**: Content is deterministic (compiled in), so the registry can be reconstructed from session capabilities on server restart.
- **Implementation**: See `crates/server/src/domains/session_files/virtual_mount_registry.rs` for `VirtualMountRegistry` and `crates/core/src/capability_types.rs` for `VirtualFileTree`.

#### Mount Application Flow

1. Session is created via POST `/v1/sessions` (with `agent_id` in body)
2. SessionService fetches agent's capabilities
3. Mounts are collected from all enabled capabilities
4. Inline mounts: files and directories are created in session filesystem
5. Virtual mounts: registered in `VirtualMountRegistry` (no DB writes)
6. Files from readonly mounts are marked `is_readonly = true`

#### Built-in Capabilities with Mounts

See `crates/test-support/src/capabilities/sample_data.rs` (the `sample_data`
demo fixture in `everruns-test-support`, EVE-875) for a concrete example of a
`mounts()` implementation; `data_knowledge` is the registered built-in that
carries mounts in product registries.

#### Platform

- **Status**: Available
- **ID**: `platform`
- **Purpose**: Expose the registered Everruns command catalog to an agent through
  the same `discover`, `query`, and `execute` contract as the `/mcp` endpoint.
- **Risk**: High. Assignment follows the admin-only capability gate; command
  execution also applies the resolved session owner's normal command policy.
- **Tools**:
  - `discover` searches authoritative command metadata and schemas.
  - `query` runs bounded Bashkit scripts with read-only commands only.
  - `execute` runs bounded Bashkit scripts with the full scriptable catalog.
- **Scope**: Tool schemas never accept `organization_id`. The in-process adapter
  is bound to the session owner and org. The distributed adapter sends the
  session and org to the server, which reloads the session, resolves its owner,
  and constructs the authenticated command context. The worker cannot nominate
  a user or elevate the caller.
- **Command contract**: MCP and capability adapters share catalog search,
  positional rewriting, script limits, output formatting, and safe error
  classification in `crates/server/src/services/platform_command_surface.rs`.
  Multi-match searches omit schemas and return a refinement hint so catalog
  browsing stays below model/tool-output limits. An exact command-name search
  returns only that operation with `bash_usage`, rendered from the registered
  input schema with exact flag names and JSON-text placeholders for array/object
  values, plus bounded `output_fields` paths for `jq`. Expanded schemas remain
  present while the operation fits the discovery response limit; beyond that
  limit they are omitted with an explicit notice while the scripting summaries
  remain authoritative. Builtins do not implement `--help`; callers use exact
  discovery instead. Registered domain commands remain the source of truth and
  execute through `Command::run`.
- **Resource-grounding boundary**: `discover` searches operation metadata, not
  entity instances. Resource-oriented phrases rank the bounded read operations
  for matching domain families, and every result identifies its scope as the
  operation catalog. A zero-match result is never evidence that an entity is
  absent. Entity existence and state come only from authorized list/get commands
  run through `query`. Integration preflight keeps installation, lifecycle
  availability, Agent/Harness attachment, provider availability, and the
  current user's connection state as separate facts.
- **Connection reads**: The scriptable catalog exposes sanitized current-user
  connection state and org-scoped provider discovery. Connection results never
  include tokens, credentials, provider metadata, or cross-user records; OAuth
  providers contributed by installed plugins remain discoverable through their
  org-scoped anchors even though those anchors are not runtime capabilities.
- **Flag validation**: Before dispatch, scripting rejects every parameter not
  declared by the command's composed input schema and reports the supported
  flags. Unknown or guessed mutation flags cannot be silently ignored, which is
  especially important for authentication and authorization configuration.
- **Dependent resources**: Scripted MCP create/read results add a derived
  `capability_ref` (`mcp:<uuid>`) beside the public MCP resource ID. The value is
  non-secret and lets a single `execute` script attach the newly registered MCP
  server to an Agent without guessing an ID conversion. The same recursive
  decoration applies to list/paginated command results.
- **Focused tool surface**: The capability has no mounts or capability
  dependencies. Platform Chat receives only catalog-backed Platform tools and
  its loop/error/compaction safeguards, so documentation browsing cannot
  displace the requested management workflow.

The built-in `platform-chat` harness inherits from the empty Base harness and
uses `platform` plus loop/error/compaction safeguards. It intentionally excludes
Generic's Bash, web, session-secret, and session-schedule surfaces: those tools
distracted catalog execution and allowed credentials or schedules to land in
the management session instead of the worker Agent. Its prompt tells the model
to discover unknown operations, inspect with `query`, mutate only when requested
with `execute`, and validate afterward. Recurring autonomous work is represented
by an Agent plus an Agent Trigger; Platform Chat must not schedule its own
session.

The surface deliberately provides commands, not a persisted provisioning plan.
Models compose the authoritative operations directly. Multi-command `execute`
scripts are not transactional: callers must inspect state after a partial
failure and avoid blindly repeating mutations.

Behavioral validation lives in `evals/platform-capability`. Its live-server
Mira study grades Platform command arguments, mutation safety, tool-call budgets,
and persisted state. The provisioning regression resolves `gpt-5.6-terra`,
binds an encrypted dummy MCP credential to the created Agent, and verifies that
hourly autonomy is owned by an Agent Trigger rather than Platform Chat's
Session. Resource-grounding coverage verifies a bounded plugin/capability/Agent/
connection preflight. Manual UI coverage remains in the Platform Chat cases
under `test_cases/agents/platform_chat/`.

#### PlatformManagement (compatibility)

- **Status**: Available
- **ID**: `platform_management`
- **Purpose**: Legacy handwritten management tools retained for compatibility
  with existing custom agents and harnesses. New built-ins use `platform`.
- **System Prompt**: Describes available management tools, common workflows, and platform docs availability
- **Mounts**: `/docs` — Everruns platform documentation (virtual, readonly). The stored/normalized mount path is `/docs`; agents/tools access it as `/workspace/docs`. Embedded at compile time via `include_dir!` from the repo `docs/` directory. Only markdown files (`.md`, `.mdx`) are included in the virtual tree.
- **Lifecycle Parity**: Must enforce the same archive/delete, assignment, and read-only rules as the public API and UI. Agents using these tools may not bypass lifecycle restrictions.
- **Tools**: Read/write split — read tools (`read_capabilities`, `read_harnesses`, `read_agents`, `read_sessions`, `session_context_report`, `session_read_messages`, `session_read_response`) return single item by ID, filtered lists, or latest session context usage; write tools (`manage_harnesses`, `manage_agents`, `manage_sessions`, `session_send_message`) perform mutations. See `crates/core/src/capabilities/platform_management.rs` for full tool parameter definitions.

Lifecycle rules for platform management:
- `delete` archives.
- Archived entities can be read but not updated or assigned.
- Deleted entities should behave as missing except where historical references are intentionally rendered as tombstones.
- Future: `destroy` (hard delete) will require a dedicated `PlatformStore` method and dangerous permission gate.

##### Design Decision: Context-Aware Tools

All platform management tools require session context to access the
`PlatformStore`. Each tool implements `requires_context() -> true` and uses
`execute_with_context()`. The host installs a typed `PlatformStoreExt` in the
neutral `ToolContextExtensions` bag; core never names the hosted store.

##### Design Decision: UI Links

All tool results include `ui_link` fields pointing to the relevant UI page (e.g., `/harnesses/{id}`, `/agents/{id}`, `/sessions/{id}/chat`). This lets agents direct users to the web interface for visual management.

##### Design Decision: Read/Write Tool Split

Platform management tools are split into read tools (`read_*`) and write tools (`manage_*`). Read tools accept an optional `id` parameter: when provided they return a single detailed item, otherwise they return a filtered list. This separation lets LLMs use read tools freely (no side effects) while write tools are clearly mutation-oriented. Session interaction is split into three single-purpose tools (`session_send_message`, `session_read_messages`, `session_read_response`) eliminating the operation-dispatch pattern for session I/O.

##### Design Decision: Capability Discovery via read_capabilities

The `read_capabilities` tool enables agents (particularly the Platform Chat) to discover available capabilities before creating or updating agents/harnesses. It queries the `PlatformStore.list_capabilities()` method which returns built-in capabilities, MCP server capabilities, and skill capabilities. The search parameter supports case-insensitive filtering across name, description, category, and ID. The Platform Chat harness system prompt instructs the agent to use `read_capabilities` before creating agents.

##### Design Decision: PlatformStore Trait

The `PlatformStore` trait and its management capabilities live in
`everruns-platform`. `DirectPlatformStore` (in `everruns-server`) implements it
using the existing `StorageBackend` and `SessionService`; core carries only the
type-keyed extension seam and the narrow neutral subagent delegate contract.

##### Design Decision: Authorization Happens In Tool Execution, Not Harness Removal

Both `platform` and compatibility `platform_management` tools must enforce
authorization using the session owner's caller context plus the active
`PermissionResolver`. Do not fix permission bugs by removing platform access
from Platform Chat; repair the execution boundary instead.

### Experimental Capabilities

Experimental capabilities are available in development environments only (`DeploymentGrade::Dev`). They may change significantly or be removed.

#### Lua execution (`lua`) and code-mode routing (`lua_code_mode`)

- `lua` — sandboxed Lua 5.4 interpreter over the session workspace. High risk,
  admin-gated, `FEATURE_LUA` + `lua` cargo feature. See
  [lua-execution.md](lua-execution.md).
- `lua_code_mode` — depends on `lua`; hides code-mode-eligible tools from the
  model's direct tool list (via a `ToolDefinitionHook`) so the agent
  orchestrates them inside a `lua` script through `tools.<name>(args)`. This is
  the canonical example of using the tool-filtering extension point to reshape
  the agent's action surface. See
  [lua-execution.md](lua-execution.md#code-mode-routing-capability-lua_code_mode).

#### DockerContainer

- **ID**: `docker_container` (Dev only, integration plugin)
- **Crate**: `integrations/docker/` (auto-registered via `inventory`, force-linked — see [architecture.md](../foundations/architecture.md#integration-plugin-force-linking))
- **Purpose**: Run commands and manage files in a session-scoped Docker container
- **Tools**: `docker_exec`, `docker_read_file`, `docker_write_file`, `docker_logs`, `docker_stop`
- **Container Lifecycle**: Lazily started on first use, persists for session, named `everruns-{session_id}`

##### Design Decision: Session-Scoped Containers

Each session gets its own isolated Docker container named `everruns-{session_id}`. This provides:
- Isolation between sessions
- Clean state on session creation
- Automatic cleanup capability via `docker_stop`

##### Design Decision: Lazy Container Start

Containers are not started until the first tool use (`docker_exec`, `docker_read_file`, or `docker_write_file`). This avoids resource waste for sessions that don't use container tools.

##### Design Decision: Dev-Only Availability

This capability is experimental and only available in development environments due to:
- Security implications of host networking
- Resource management considerations
- API stability concerns

### Message Filters

Capabilities can contribute message filters that modify how messages are retrieved from the database. This enables features like:

- **Time-based filtering**: Load messages from a specific time range
- **Event type filtering**: Filter by message type (user, agent, tool result)
- **Tool name filtering**: Filter tool results by specific tool names
- **Full-text search**: Search message content
- **Ephemeral message injection**: Add system reminders or summaries without persistence

#### How Message Filters Work

1. **Filter Contribution**: Capabilities implement `message_filter_provider()` returning a `MessageFilterProvider`
2. **Priority Ordering**: Providers are sorted by priority (lower = earlier)
3. **Query Building**: Filters are combined into a `MessageQuery` and applied to message retrieval
4. **DB Mapping**: Most filters map directly to SQL WHERE clauses for efficiency
5. **In-Memory Fallback**: Custom predicates use Rust closures for complex filtering

#### MessageFilter Types and Provider Trait

For the full `MessageFilter` enum variants (TimeRange, EventTypes, ToolName, Search, ExcludeIds, IncludeIds, Custom) and `MessageFilterProvider` trait, see `crates/core/src/message_filter.rs`. Most filters map directly to SQL WHERE clauses; `Custom` uses in-memory predicates applied after DB query.

#### Message Injection

Ephemeral messages can be injected into the result set without persistence (summaries, reminders, context). Injections applied after filtering at positions: `Start`, `End`, `BeforeIndex(n)`, `AfterIndex(n)`.

#### Filter Application Flow

1. **Collect Providers**: During capability resolution, filter providers are collected from enabled capabilities
2. **Sort by Priority**: Providers are sorted (lower priority values first)
3. **Build Query**: Each provider modifies the `MessageQuery` in priority order
4. **Execute Query**: The query is executed against the message store
5. **Apply Custom Filters**: In-memory predicates filter the results
6. **Apply Injections**: Ephemeral messages are inserted at their specified positions

#### Design Decisions

| Question | Decision |
|----------|----------|
| Where are filters defined? | Capabilities implement `message_filter_provider()` |
| How are they combined? | AND semantics - all filters must match |
| Order of application? | By `priority()` value (lower = earlier) |
| Can filters be stacked? | Yes - multiple capabilities can each contribute filters |
| Database efficiency? | Most filters map to SQL; only `Custom` requires in-memory filtering |
| DEV_MODE parity? | In-memory storage implements the same filter semantics |

### Output Guardrails

Capabilities can contribute streaming output guardrails that inspect the model's text deltas as they arrive and, post factum, replace the assistant message when a violation is detected. This is the same extension pattern as message filters, but operates on the agent's outgoing stream rather than the message history.

#### How Output Guardrails Work

1. **Contribution**: Capabilities implement `output_guardrails()` returning a list of `Arc<dyn OutputGuardrail>` providers
2. **Arming**: At the start of each assistant message stream, `ReasonAtom` calls `OutputGuardrail::arm(ctx)` per provider. `ctx` carries the fully assembled system prompt and the contributing capability's per-agent config. Providers may return `None` to skip arming for this stream (e.g. nothing useful to derive from the prompt)
3. **Streaming check**: After each text delta is appended to the accumulated assistant text, every armed `OutputGuardrailRun::check(accumulated, delta)` runs synchronously in the streaming hot path. The contract is that `check` is cheap — substring matches, regex, hash lookups; not network calls or LLM inference
4. **Block decision**: A guardrail returns `GuardrailDecision::Block { reason_code, replacement }` to abort the stream. The pending (unsuppressed) delta is dropped, the LLM stream is dropped, and `output.message.replaced` is emitted. The replacement text becomes the assistant message body in `output.message.completed`; the original tokens are never persisted or replayed on subsequent turns
5. **Pass decision**: `GuardrailDecision::Pass` lets streaming continue. The next delta runs the check again

#### Streaming Timeline With a Trip

```
output.message.started
        │
        ▼
output.message.delta  ← (model text accumulating)
output.message.delta
        │
        ▼            (guardrail trips on the next delta — pending delta is suppressed)
output.message.replaced  ← UI discards accumulated text, shows replacement
        │
        ▼
output.message.completed  ← persisted message body = replacement
```

#### Trait + Helpers

See `crates/core/src/output_guardrail.rs` for `OutputGuardrail`, `OutputGuardrailRun`, `OutputGuardrailContext`, `GuardrailDecision`, `GuardrailBlock`, `ArmedGuardrail`, and the `evaluate_guardrails` helper used by `ReasonAtom`.

#### Design Decisions

| Question | Decision |
|----------|----------|
| Sync vs async `check`? | Sync. Guardrails run on every batched delta in the hot path; slow checks would throttle the stream. Heavy guardrails (e.g. an LLM moderator) must run elsewhere |
| Where do guardrails live? | On `Capability` trait, not `RuntimeAgent`. They are derived from `resolved_capability_configs` at execution time so `RuntimeAgent` stays serializable |
| What surface is checked? | Assistant text only (`output.message.delta`). `tool.output.delta` and `reason.thinking.delta` are out of scope for v1 — easy to extend later by adding a surface enum |
| Original tokens persisted? | No. After replacement, every downstream event (`llm.generation`, `output.message.completed`) carries the replacement text. The model itself never sees the leak on subsequent turns |
| What if multiple guardrails trip? | The first to return `Block` (in capability registration order) wins. Subsequent guardrails are not consulted on a blocked stream |
| Per-stream state? | Yes. `arm()` returns a fresh `Box<dyn OutputGuardrailRun>` per stream so guardrails can hold cursors, dedup tables, etc. without sharing across sessions |

### Extension Points (Future)

1. **Custom Capabilities**: User-defined capabilities with custom tools
2. **Conflict Resolution**: Handle tool name conflicts between capabilities
3. **Capability Versioning**: Track capability API versions for compatibility
4. **Dynamic Mount Sources**: External file sources (URLs, S3, etc.)
5. **OR Filter Semantics**: Support for OR combinations of filters
6. **Guardrail Surfaces**: Extend output guardrails to `tool.output.delta` and `reason.thinking.delta` via a surface enum on `OutputGuardrail`
