# Framework and host domain language

The vocabulary used by the `everruns` Framework and advanced `everruns-host`
composition. These concepts are shared with the worker- and server-backed
runtime, so they transfer to the rest of Everruns.

## Composition

- **HostComposition**: the shared runtime bundle consumed by the runtime,
  server, and worker alike. Owns the capability registry, LLM driver registry,
  connection-provider registry, built-in harness templates, and the
  `SessionFileSystemFactory`. Lives in `everruns-core` so any binary can build
  one. `HostComposition::new(capabilities, drivers)` is the minimal form.

- **Harness**: the reusable agent template: a stable `name`, optional system
  prompt, a set of capabilities, optional MCP servers, and config. Harnesses
  can **chain** (a child harness overlays a parent). The system prompt is
  optional, a harness may contribute only capabilities or MCP servers.

- **Agent**: a configuration overlay on a harness (instructions, model
  selection, max iterations, …). Folded over the harness chain via
  `AgentConfigOverlay` to produce the effective configuration.

- **Session**: a single conversation / run. Holds message history and a
  workspace (virtual filesystem). Created from a harness + agent.

- **Capability**: a registered tool or behavior the agent can use, addressed by
  name (e.g. `session_file_system`, `agent_instructions`, `skills`,
  `infinity_context`, `duckduckgo`). Built-ins live in `everruns-core`; optional
  ones ship as integration crates. Active capabilities are resolved from the
  effective harness/agent/session overlay, **not** the full platform registry.

- **Driver**: an LLM provider implementation, keyed by a `DriverId`
  (`OpenAI`, `Anthropic`, `Gemini`, `OpenRouter`, `Mai`, `Bedrock`, `LlmSim`,
  `OpenAICompletions`, …). Registered in a `DriverRegistry`.

- **ResolvedModel**: a concrete model selection: `model` name, `provider_type`
  (`DriverId`), `api_key`, optional `base_url`, optional `provider_metadata`.
  The runtime requires a **default model**; `build()` fails without one.

## Execution

- **Turn**: one unit of execution: `input → reason → act`. Driven by
  `run_turn(session_id, input)` or the `run_text_turn(session_id, text)`
  convenience.

- **Engine phases**: the shared execution units the immediate and durable drivers both run:
  `InputAtom` (records the input message, emits `input.message`), `ReasonAtom`
  (assembles context, calls the LLM), `ActAtom` (executes tool calls). Atom
  algorithms live in `everruns-engine`; host composition injects their effect
  contracts so improvements flow to both embedded and durable hosts.

- **TurnExecution**: the serializable `everruns-engine` state machine advanced
  by every execution driver.

- **Assembled turn context**: the merged context the reason phase uses: harness
  chain + agent + session overlay, resolved capabilities (with dependency
  resolution and message filtering), resolved model and locale, and the
  constructed `RuntimeAgent`. Built by `everruns_core::assemble_turn_context(...)`
  at execution time and `inspect_turn_context(...)` for read-only inspection
  (`InProcessRuntime::load_context`).

- **RuntimeAgent**: the fully-resolved agent the LLM step runs against: final
  system prompt, model, and tool set.

## Framework application surface

- **Agent / AgentBuilder**: value-first application configuration for
  instructions, models, tools, files, workspaces, MCP, plugins, and policies.

- **Session**: isolated multi-turn execution with events, cancellation,
  context inspection, work, wakes, and schedules.

- **Model / ModelSpec / Provider**: credential-free model identity paired
  with a provider implementation or convenience.

## Advanced host surface

- **InProcessRuntimeBuilder**: the `everruns-host` low-level entrypoint. Set the
  `HostComposition`, register extra capabilities/drivers, seed
  harnesses/agents/sessions, configure the default model, optionally register
  `llmsim`, and swap stores via `HostBackends`.

- **HarnessBuilder / AgentBuilder / SessionBuilder**: supported construction
  path for seed models. Prefer these over raw struct literals so new optional
  core fields can be defaulted inside the host crate.

- **single_session(...)**: compact path for the common one-harness, one-agent,
  one-session shape. Records the generated id for `default_session_id()`.

- **InProcessRuntime**: the built runtime. Key methods: `run_turn`,
  `run_text_turn`, `default_session_id`, `load_context`, `messages`,
  `read_file`, `events`.

- **HostBackends**: the bundle of overridable stores. Start from
  `HostBackends::in_memory()` and add canonical event, provider, filesystem,
  task, schedule, platform, or connection-resolver contracts as needed.

## Host execution (for durable / server-backed embedders)

`everruns-host` owns a reusable contract so durable or server-backed hosts reuse
phase orchestration instead of duplicating atom wiring:

- **RuntimeHostAdapter** / **RuntimeSessionLifecycle**: host boundaries.
- **execute_input_activity / execute_reason_activity / execute_act_activity**,
  phase-local orchestration entrypoints.
- **advance_host_execution(...)** with an engine **Execution** driver,
  durable-agnostic turn-strategy advancement. The engine decides the next
  semantic step; the host owns queueing, retries, scheduling, persistence, and
  resumption. There is no stateless compatibility planner.

`InProcessRuntime` itself implements `RuntimeHostAdapter` and drives its own turn
loop by calling these canonical host activity functions directly.
