# Proposal: Platform capability backed by the Everruns command surface

Status: implemented command surface. Agent-scoped credential binding and
runtime-loop hardening remain separate follow-ups. Follow-up to
[`platform-chat-yolop-parity-investigation.md`](platform-chat-yolop-parity-investigation.md).
Validated against `origin/main` at `afe3a8ea6ffe9d09cc3cea4bb0971c090b0eca20`.

## Decision

Add a built-in `platform` capability that exposes the same catalog-backed
management interface as the Everruns MCP endpoint:

- `discover`
- `query`
- `execute`

The model discovers authoritative command names and schemas, uses `query` for
read-only inspection, and uses `execute` for mutations. There is no
intermediate workflow state or growing collection of bespoke Platform Chat
CRUD tools.

Both surfaces must share one implementation:

```text
Everruns MCP ───────┐
                    ├─> command catalog + ScriptedTool + Command::run
Platform capability ┘
```

The command inventory is already the source of truth for operations such as
`list_models`, `create_mcp_server`, `create_agent`, and
`create_agent_trigger`. Exposing that inventory to Platform Chat solves the
surface drift that caused the incident.

## Why this is enough

The existing domain commands already own the behavior Platform Chat was
missing:

- model listing and lookup;
- MCP server CRUD and tool refresh;
- Agent creation with `default_model_id`, harness, capabilities, and scoped MCP
  configuration;
- Agent Trigger CRUD and manual invocation;
- plugin, capability, App, session, and other platform operations.

The current `platform_management` capability re-expresses a small subset of
those commands through handwritten tools and a separate `PlatformStore` trait.
That adapter has drifted: for example, its Agent tool omits the model field and
advertises a capability update it cannot execute. The MCP command surface does
not have that problem because `discover`, `query`, and `execute` are generated
from the registered domain-command inventory.

Smart models can compose these primitives directly. Everruns should provide
accurate schemas, authorization, safe execution, and bounded errors—not a
second orchestration layer.

## Tool contract

### `discover`

Same behavior as Everruns MCP `discover`:

- search the registered command catalog by name, category, description, and
  schema terms;
- return matching commands with input/output schemas and output-shape hints;
- support listing the complete catalog when explicitly requested;
- describe whether each operation is read-only or mutating;
- return only commands exposed to the scripted platform surface.

This is how Platform Chat learns that the correct operations are
`list_models`, `create_agent`, `create_mcp_server`, and
`create_agent_trigger`. It never guesses `read_models` again.

### `query`

Same behavior as Everruns MCP `query`:

- run a bounded Bashkit script;
- expose only inventory commands classified as read-only;
- support variables, pipes, conditionals, loops, and `jq`;
- preserve the existing exclusions for nominally read-only commands with
  outbound or other side effects;
- return the same formatted output and sanitized errors.

One `query` call can inspect models, existing MCP servers, Agents, capabilities,
and Triggers without repeatedly calling separate list tools.

### `execute`

Same behavior as Everruns MCP `execute`:

- run a bounded Bashkit script with the full scriptable command catalog;
- dispatch every builtin through the registered domain command;
- call `Command::run`, never `Command::execute` directly;
- preserve command validation, active `PermissionResolver`, auditing,
  resource limits, error classification, and response decoration;
- support multi-command scripts so the model can create related resources in
  dependency order and then validate them.

`execute` is not atomic. If command three fails after commands one and two
succeed, the result must make that partial state clear. Platform Chat reads the
state with `query`, reuses existing resources, and reports honest links rather
than blindly retrying creation.

## Shared implementation

Today the reusable behavior is located under
`crates/server/src/api/mcp_endpoint/`:

- `catalog.rs` builds the Bashkit toolset from `CommandDescriptor` inventory;
- `positional.rs` adapts scripted positional arguments;
- `mod.rs::{tool_discover,tool_query,tool_execute}` owns the Tier-2 execution
  path;
- `tool_registry.rs` owns the MCP tool definitions.

Move the transport-neutral Tier-2 behavior into a shared server component,
tentatively `platform_command_surface`:

- catalog search and schema shaping;
- read-only/full `ScriptedTool` construction;
- positional and aggregate-argument rewriting;
- script limits and timeout enforcement;
- result decoration and sanitized error formatting;
- `discover`, `query`, and `execute` entrypoints over an authenticated command
  context.

The HTTP MCP endpoint becomes one adapter to this component. The `platform`
capability becomes another. Do not copy the catalog, schemas, allowlists,
positional rewriting, or error classifier.

### Runtime service seam

The capability runs inside the worker/runtime while the command inventory and
authorized domain context live on the server. The existing `PlatformStore`
runtime service owns the three transport-neutral methods:

```text
PlatformCommandSurface
  discover(arguments) -> structured catalog result
  query(script, timeout) -> result
  execute(script, timeout) -> result
```

Exact Rust types belong in source. The important contract is that the service
is already bound to the current Platform Chat session's org and caller; the
model cannot provide either.

Do not reuse `CommandHost`: that service handles capability slash commands and
out-of-band LLM completions, not authenticated domain-command dispatch.

- The in-process adapter calls the shared server component directly.
- The distributed worker adapter uses an internal RPC that forwards the
  operation and owning session ID to the server.
- The server reloads the session, resolves its effective human owner, builds
  the real `Caller`, and applies the active `PermissionResolver`.
- The RPC must not use `Caller::internal` for user-initiated Platform Chat
  execution.

This follows the authorization pattern already enforced by
`DirectPlatformStore`, but avoids extending its domain-by-domain CRUD methods.

### Tool definitions

Extract neutral definitions for `discover`, `query`, and `execute` so the MCP
adapter and Capability registry derive from the same owners. MCP protocol
version decoration (`title`, `outputSchema`, annotations) remains an adapter
concern; the operation names, descriptions, input constraints, and execution
limits do not fork.

Add a parity test that compares the MCP and Platform definitions after removing
only transport-specific fields.

Remote MCP tool names are server-prefixed, so they do not collide with these
built-ins. General duplicate built-in tool rejection is a registry-wide concern
and is not part of this change.

## Scope and organization behavior

Everruns MCP accepts `organization_id` because an external, stateless MCP client
can belong to multiple orgs and has no current-org session. Platform Chat
already runs inside one org-scoped session.

Therefore the Platform tools use the same methods and command surface but are
strictly bound to the session org:

- `organization_id` is omitted from Platform tool schemas;
- an injected org override is rejected;
- every command context uses the session org and session owner's caller;
- entity IDs are still revalidated under that org.

This is the only intentional semantic difference from the MCP Tier-2 tools. It
prevents a prompt-injected Platform Chat turn from using the worker boundary as
a cross-org switch.

## Capability lifecycle

Introduce built-in capability ID `platform` for the built-in Platform Chat
harness and the existing seeded Platform Manager Agent.

Migration:

1. Register `platform` alongside the existing `platform_management` capability.
2. Switch Platform Chat and the seeded Platform Manager Agent to `platform` and
   update their prompts.
3. Keep `platform_management` available temporarily for embedded users and
   compatibility tests.
4. Remove or deprecate the handwritten capability after parity and migration
   coverage prove no required Tier-1 behavior is lost.

The existing MCP convenience tools such as `agent_run`,
`session_send_message`, and `session_get_status` are not duplicated in the
first Platform capability version. Their underlying session/Agent commands are
available through `discover/query/execute`. If a hot path later needs a
dedicated tool, it should wrap the same command and be justified by measured
latency or model reliability.

## Platform Chat workflow

Update `crates/server/src/harnesses/platform_chat.rs::SYSTEM_PROMPT` to describe
three rules, without copying command schemas:

1. Use `discover` when the command name or schema is unknown.
2. Use `query` for inspection and validation.
3. Use `execute` only for requested mutations, then validate with `query`.

For the dad-joke request, the model should use the surface in this order:

1. discover the model, MCP server, Agent, and Agent Trigger operations;
2. query the model catalog and existing Visti/MCP/Agent state;
3. confirm the requested reusable mutations once;
4. execute the necessary MCP server and Agent commands in dependency order;
5. configure the Agent-owned credential through the secure path described
   below;
6. execute the Agent Trigger command last;
7. query the final Agent and Trigger state;
8. return entity links and no internal narration.

The model must use an Agent Trigger for autonomous work, never a Session
Schedule on Platform Chat.

## Credential boundary

`discover/query/execute` solve platform discovery and mutations, but they must
not carry secret values supplied through chat. The Visti channel key is a tool
argument, not standard MCP bearer authentication, and future trigger sessions
cannot read Platform Chat's session `secret_store`.

This change intentionally does not add a credential tool. A follow-up should
add one host-interactive tool alongside the three shared methods:

- `request_agent_credential`

It accepts the target Agent, MCP server, tool, argument path, label, and purpose
but no value. The authenticated UI shows the exact destination and sends the
secret directly to a credential endpoint. The tool returns only configured or
cancelled plus an opaque binding ID.

The stored binding is Agent-identity-owned. At MCP tool assembly/execution the
runtime removes the bound argument from the model-visible schema, rejects a
model override, decrypts and injects the value immediately before the outbound
call, and redacts known-value echoes before persistence. Wrong Agent, principal,
tool, endpoint, schema, or revoked credentials fail closed.

This tool is intentionally Platform-host-specific and is not forced into the
public Everruns MCP interface. An MCP client without the authenticated Everruns
UI cannot safely service the prompt; it receives a link to the normal Agent
credential page instead.

If the user already pasted a key into chat, Platform Chat must refuse to reuse
it and ask for rotation plus secure re-entry.

## Security and authorization

The `platform` capability is high impact: `execute` exposes every scriptable
mutation allowed to the caller. Preserve these boundaries:

- capability assigned by default only to the Platform Chat harness and seeded
  Platform Manager Agent;
- session owner, org, and custom permission resolver re-established server-side
  for every call;
- each builtin dispatched through `Command::run`;
- read-only `query` catalog generated from authoritative command metadata plus
  the current side-effect exclusions;
- Bashkit command, loop, AST, input, output, and timeout limits identical to
  Everruns MCP;
- internal errors sanitized before reaching the model;
- `execute` declared mutating/open-world so narration and any future tool
  approval treat it accordingly;
- scripts and command names recorded for audit, with secret values prohibited
  and redacted;
- no cross-org override and no internal-caller authorization bypass.

`execute` can batch several mutations. Domain policy still evaluates per
command, so a script stops at the unauthorized command without broadening the
caller's grants.

## Runtime and cost controls

The generic surface greatly reduces tool count, but retain incident hardening:

- enable `progress_guard` for Platform Chat;
- enable `prompt_caching` after Anthropic trace coverage;
- set a Platform Chat-specific iteration/tool/cost ceiling below Generic's
  500-iteration default;
- treat repeated identical `discover`/`query` scripts and unchanged results as
  no progress;
- instruct the model to run one decisive validation query rather than repeated
  catalog scans;
- keep final output separate from commentary through prompt/eval coverage.

## Target end-to-end scenario after the credential follow-up

Given an authorized user and no Visti MCP server:

1. `discover` returns the real schemas for the necessary registered commands.
2. `query` finds the exact GPT-5.6 Terra model and confirms Visti/Agent/Trigger
   do not already exist.
3. Platform Chat asks once for the requested shared mutations.
4. `execute` registers `https://visti.sh/mcp` and creates the Agent with the
   exact model and returned MCP virtual capability.
5. `request_agent_credential` securely binds the rotated channel key to that
   Agent's `visti_send.channel_key` destination without exposing the value.
6. `execute` creates the hourly `session_per_invocation` Agent Trigger.
7. `query` verifies the Agent model/capability and enabled Trigger.
8. The final answer links the resources and contains no internal narration.
9. No notification is sent until the Trigger fires or the user explicitly
   invokes the existing manual-trigger command.

No `read_models` guess, repeated capability search, Bash networking workaround,
Platform Chat Session Schedule, or cross-session secret copy occurs.

## Tests

### Shared-surface parity

- MCP and Platform `discover/query/execute` definitions match after documented
  transport-specific fields are removed;
- both adapters return the same catalog entries, output shapes, positional
  rewriting, structured parameters, decorations, and sanitized errors;
- every inventory command appears or is excluded identically;
- read-only exclusions remain identical;
- catalog/schema-hash changes invalidate both surfaces together.
- duplicate built-in tool names fail capability assembly deterministically.

### Authorization and isolation

- Platform calls execute as the session owner under default and custom
  `PermissionResolver` implementations;
- member/admin distinctions match HTTP and MCP command execution;
- `organization_id` is absent and injection is rejected;
- cross-org entity IDs fail;
- distributed RPC cannot select `Caller::internal`;
- a multi-command execute script stops on the first denied command and does not
  run later statements when fail-fast shell behavior is requested.

### Script execution

- query exposes only read-only builtins;
- execute exposes the full scriptable catalog;
- time, command, loop, AST, input, and output limits match MCP;
- partial mutation output is preserved and actionable;
- model/MCP/Agent/Trigger commands work through the generic catalog without
  bespoke Platform tools.

### Credential and end to end (follow-up)

- secret value is absent from chat, tool calls/results, events, logs, traces,
  audit payloads, and ATIF export;
- mock Visti receives the injected value while the model-visible tool schema
  omits it;
- wrong-Agent, override, revocation, and schema-drift cases fail closed;
- the full Terra + Visti + Agent Trigger flow completes in a bounded number of
  model iterations and produces one clean final answer;
- the scheduled session runs the created Agent, not Platform Chat.

## Delivered boundary

This implementation combines the three inseparable command-surface layers in
one PR: shared MCP execution, direct/distributed runtime adapters, and the
capability that consumes them. Splitting them would leave dead transport APIs
or a capability that cannot execute.

Separate follow-up PRs:

1. **Add Agent credential binding.** Secure UI prompt, Agent-identity encrypted
   credential, MCP argument schema rewrite/injection/redaction, and threat-model
   tests.
2. **Land the Visti eval and hardening.** Full autonomous-Agent scenario,
   progress guard, prompt caching traces, Platform ceilings, and output-quality
   evals.

The credential PR is the only new secret-handling architecture. The generic
Platform capability removes the need for separate model, MCP, Agent, plugin,
and Trigger tool PRs.

## Success criteria

- Platform Chat manages the full command inventory through the same
  `discover/query/execute` behavior as Everruns MCP.
- A newly registered domain command becomes available to both surfaces without
  another Platform adapter change.
- The Terra + hourly Agent request can discover and create the correct model,
  Agent, MCP, and Trigger resources without guessed tools or a second
  orchestration subsystem. Visti credential injection remains gated on the
  separate Agent-scoped credential follow-up.
- Domain permissions and org isolation match HTTP/MCP behavior.
- No credential byte enters model-visible or persisted conversation data.
- `platform_management` can be deprecated after compatibility coverage, ending
  the handwritten adapter drift.
