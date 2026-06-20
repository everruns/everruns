# Optional crates for everruns-runtime

`everruns-runtime` is provider- and tool-neutral: you add only the crates you
use. This catalog lists the optional crates that extend an in-process runtime,
how to register them, and what they contribute.

The two required crates are always:

- `everruns-runtime` — the in-process engine ([docs.rs](https://docs.rs/everruns-runtime))
- `everruns-core` — shared types (capabilities, drivers, models, `PlatformDefinition`)

## LLM provider crates (drivers)

Each provider crate registers a driver keyed by a `DriverId`. Register the
driver, then point `default_model.provider_type` at the matching `DriverId`.

| Crate | `DriverId` | Notes |
|---|---|---|
| `everruns-openai` | `DriverId::OpenAI` (Responses API), `DriverId::OpenAICompletions` (Chat Completions) | Also covers Azure OpenAI and OpenAI-compatible endpoints via `base_url` |
| `everruns-anthropic` | `DriverId::Anthropic` | Supports prompt caching |
| `everruns-gemini` | `DriverId::Gemini` | Google Gemini |
| `everruns-openrouter` | `DriverId::OpenRouter` | OpenRouter model gateway |
| `everruns-mai` | `DriverId::Mai` | — |
| `everruns-bedrock` | `DriverId::Bedrock` | AWS Bedrock |
| built into `everruns-core` | `DriverId::LlmSim` | Deterministic simulator for tests/examples; no API key |

Registration pattern:

```rust,ignore
let mut drivers = DriverRegistry::new();
everruns_openai::register_driver(&mut drivers);
let platform = PlatformDefinition::new(capabilities, drivers);
```

For an OpenAI-compatible local server (e.g. Ollama, a gateway), keep the
matching `DriverId` and set `base_url` on the `ResolvedModel`.

## Integration crates (capabilities / tools)

Integration crates ship optional capabilities. Construct the capability and
`register` it on a `CapabilityRegistry`, then reference it by name on a harness.

### Sandbox / compute

| Crate | Capability | Notes |
|---|---|---|
| `everruns-integrations-daytona` | `DaytonaCapability` | Daytona cloud sandbox. Connection-aware — see "Connection-aware integrations" below |
| `everruns-integrations-e2b` | E2B sandbox capability | E2B cloud sandbox |
| `everruns-integrations-docker` | Docker capability | Local Docker container sandbox |
| `everruns-integrations-deno` | Deno capability | Deno sandbox |
| `everruns-integrations-sprites` | Sprites capability | Sprites cloud sandbox |
| `everruns-integrations-cursor` | Cursor capability | Cursor Cloud Agents |

### Search / web

| Crate | Capability | Notes |
|---|---|---|
| `everruns-integrations-duckduckgo` | `DuckDuckGoCapability` | Free web search (`duckduckgo_search`); **no API key** |
| `everruns-integrations-brave-search` | Brave Search capability | Requires Brave API key |
| `everruns-integrations-parallel` | Parallel capability | Parallel web search MCP |
| `everruns-integrations-browserless` | Browserless capability | Headless browser automation |
| `everruns-integrations-ard` | ARD capability | Agentic Resource Discovery client |

### Blueprints

| Crate | Contributes | Notes |
|---|---|---|
| `everruns-integrations-github` | Agent blueprints | GitHub-backed agent blueprints |

Registration pattern:

```rust,ignore
let mut capabilities = CapabilityRegistry::new();
capabilities.register(DuckDuckGoCapability);
capabilities.register(DaytonaCapability);
let platform = PlatformDefinition::new(capabilities, drivers);
```

## Connection-aware integrations (credentials)

Some integrations — Daytona is the canonical example — need a per-user
credential resolved at tool-execution time. Supply a credential source through
`RuntimeBackends`:

```rust,ignore
let backends = RuntimeBackends::in_memory()
    .with_connection_resolver(my_connection_resolver); // Arc<dyn UserConnectionResolver>
```

The runtime then exposes it via `ToolContext.connection_resolver`. There is no
in-memory default: a resolver implies a real credential source the embedder
owns. When unset, connection-aware tools fall back to their own guidance
(session secret, then "connect the provider"). See
`crates/server/specs/user-connections.md` in the repo.

## Durable local backends

| Crate | Provides |
|---|---|
| `everruns-local` | SQLite-backed, restart-survivable stores for single-process hosts: `LocalSessionTaskRegistry`, `LocalScheduleStore`, `LocalPlatformStore`, a `LocalProfile`, a composable `LocalBackends`, and an optional `LocalRuntimeBuilder` |

`everruns-local` populates the runtime's optional host-backend slots
(`session_task_registry`, `schedule_store_factory`, `platform_store_factory`),
which otherwise default to in-memory. Use it when background tasks, schedules, or
session state must survive a process restart.

## Runtime crate features

| Feature | Effect | Default |
|---|---|---|
| `mcp-stdio` | Enables local-process (stdio) MCP servers | off |
| `lua` | Compiles the experimental sandboxed Lua execution engine (`lua` / `lua_code_mode` capabilities) | off |

```bash
cargo add everruns-runtime --features mcp-stdio
```

## Backend store override traits (in `everruns-runtime`)

Supply your own implementation of any of these via `RuntimeBackends`; anything
not overridden stays in-memory:

`RuntimeHarnessStore`, `RuntimeAgentStore`, `RuntimeSessionStore`,
`RuntimeMessageStore`, `RuntimeProviderStore`, `EventBus`. Session files come
from the platform's `SessionFileSystemFactory`
(`InMemorySessionFileSystemFactory`, `RealDiskSessionFileSystemFactory`, or a
custom factory such as S3).
