# Focused crates for Framework applications and advanced hosts

Start with `everruns`, then add only the provider, integration, or host crates
you use. This catalog lists focused crates, how to register them, and what they
contribute.

The primary and advanced-host entrypoints are:

- `everruns`, application-facing Framework entrypoint ([docs.rs](https://docs.rs/everruns))
- `everruns-host`, low-level host composition for advanced integrators ([docs.rs](https://docs.rs/everruns-host))

## LLM provider crates (drivers)

Each provider crate registers a driver keyed by a `DriverId`. Register the
driver, then point `default_model.provider_type` at the matching `DriverId`.

| Crate | `DriverId` | Notes |
|---|---|---|
| `everruns-openai` | `DriverId::OpenAI` (Responses API), `DriverId::AzureOpenAI` (Azure OpenAI Responses API), `DriverId::OpenAICompletions` (Chat Completions) | One `register_driver` call registers all three; also reaches OpenAI-compatible endpoints via `base_url` |
| `everruns-anthropic` | `DriverId::Anthropic` | Supports prompt caching |
| `everruns-gemini` | `DriverId::Gemini` | Google Gemini |
| `everruns-openrouter` | `DriverId::OpenRouter` | OpenRouter model gateway |
| `everruns-fireworks` | `DriverId::Fireworks` | Fireworks AI, open-model inference (Llama, Qwen, DeepSeek, GLM, …) |
| `everruns-mai` | `DriverId::Mai` | Microsoft MAI |
| `everruns-bedrock` | `DriverId::Bedrock` | AWS Bedrock |
| built into `everruns-core` | `DriverId::LlmSim` | Deterministic simulator for tests/examples; no real API key |

Registration pattern:

```rust,ignore
let mut drivers = DriverRegistry::new();
everruns_openai::register_driver(&mut drivers);
let platform = HostComposition::new(capabilities, drivers);
```

For an OpenAI-compatible local server (e.g. Ollama, a gateway), keep the
matching `DriverId` and set `base_url` on the `ResolvedModel`.

## Integration crates (capabilities / tools)

Integration crates ship optional capabilities. Construct the capability and
`register` it on a `CapabilityRegistry`, then reference it by name on a harness.

### Sandbox / compute

| Crate | Capability | Notes |
|---|---|---|
| `everruns-integrations-daytona` | `DaytonaCapability` | Daytona cloud sandbox. Connection-aware, see "Connection-aware integrations" below |
| `everruns-integrations-e2b` | E2B sandbox capability | E2B cloud sandbox |
| `everruns-integrations-docker` | Docker capability | Local Docker container sandbox |
| `everruns-integrations-deno` | Deno capability | Deno sandbox |
| `everruns-integrations-sprites` | Sprites capability | Sprites cloud sandbox |
| `everruns-integrations-cursor` | Cursor capability | Cursor Cloud Agents |

### Search / web

| Crate | Capability | Notes |
|---|---|---|
| `everruns-integrations-duckduckgo` | `DuckDuckGoCapability` | Free instant answers (`duckduckgo_instant_answer`), not full web search; **no API key** |
| `everruns-integrations-brave-search` | Brave Search capability | Requires Brave API key |
| `everruns-integrations-parallel` | Parallel capability | Parallel web search MCP |
| `everruns-integrations-browserless` | Browserless capability | Headless browser automation |

### Discovery

Platform-level discovery capability (a sibling of MCP / A2A, not an external
integration).

| Crate | Capability | Notes |
|---|---|---|
| `everruns-ard` | `resource_discovery` | Agentic Resource Discovery client, discover + attach external MCP servers / A2A agents at runtime |

### Blueprints

| Crate | Contributes | Notes |
|---|---|---|
| `everruns-integrations-github` | Agent blueprints | GitHub-backed agent blueprints |

Registration pattern:

```rust,ignore
let mut capabilities = CapabilityRegistry::new();
capabilities.register(DuckDuckGoCapability);
capabilities.register(DaytonaCapability);
let platform = HostComposition::new(capabilities, drivers);
```

## Connection-aware integrations (credentials)

Some integrations, Daytona is the canonical example, need a per-user
credential resolved at tool-execution time. Advanced hosts supply a credential
source through `HostBackends`:

```rust,ignore
let backends = HostBackends::in_memory()
    .with_connection_resolver(my_connection_resolver); // Arc<dyn UserConnectionResolver>
```

The runtime then exposes it via `ToolContext.connection_resolver`. There is no
in-memory default: a resolver implies a real credential source the embedder
owns. When unset, connection-aware tools fall back to their own guidance
(session secret, then "connect the provider"). See
`crates/server/specs/user-connections.md` in the repo.

## Durable local backends

The `everruns::local` module (feature `local`) provides SQLite-backed,
restart-survivable stores for single-process hosts: `LocalSessionTaskRegistry`,
`LocalScheduleStore`, `LocalPlatformStore`, `LocalProfile`, composable
`LocalBackends`, and `LocalRuntimeBuilder`.

The module populates optional `everruns-host` backend slots
(`session_task_registry`, `schedule_store_factory`, `platform_store_factory`),
which otherwise default to in-memory. Use it when background tasks, schedules, or
session state must survive a process restart.

## Framework and host features

| Feature | Effect | Default |
|---|---|---|
| `mcp-stdio` | Enables local-process (stdio) MCP servers | off |
| `lua` on `everruns-host` | Compiles the experimental sandboxed Lua execution engine for advanced hosts | off |

```bash
cargo add everruns --features mcp-stdio
```

## Backend store override traits (in `everruns-host`)

Supply your own implementation through `HostBackends`; anything
not overridden stays in-memory:

`RuntimeHarnessStore`, `RuntimeAgentStore`, `RuntimeSessionStore`,
`RuntimeProviderStore`, `EventLog`, `EventSink`, and `EventReader`. Session files come
from the platform's `SessionFileSystemFactory`
(`InMemorySessionFileSystemFactory`, `RealDiskSessionFileSystemFactory`, or a
custom factory such as S3).
