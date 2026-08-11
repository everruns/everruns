# Investigation: failed Platform Chat autonomous-agent provisioning

Status: evidence-backed investigation and PR plan. This is intentionally not a
port of Yolop. The incident crosses independent control-plane, credential,
scheduling, and runtime-control boundaries, so no single safe PR-sized root fix
exists.

## Baselines and primary evidence

The source baselines were fetched before analysis:

- Everruns `origin/main`: `afe3a8ea6ffe9d09cc3cea4bb0971c090b0eca20`
  (`docs(guardrails): add capability page and refresh gallery blurb (#2979)`).
  The findings and proposal were revalidated after the intervening changes; the
  command and Platform Chat behavior described below is unchanged.
- Yolop `origin/main`: `ab206f533965a270d996493937d3efbdee990ee7`
  (`refactor(runtime): adopt everruns 0.17.23 primitives (#544)`).

The primary trace is Everruns session
`session_019fd99697c5728285b44ddc10b51df8`, turn
`turn_019fd9c9c1ea75d3a6aeb6c8f9890783`, from
2026-08-07 01:15:06Z through 01:22:39Z. `turn.completed` records:

| Signal | Value |
|---|---:|
| Iterations / LLM calls | 30 |
| Tool calls | 42 |
| Duration | 454,454 ms |
| Input / output tokens | 510,197 / 23,434 |
| Cache read / creation tokens | 0 / 0 |
| Effective cost | $3.136835 |

The tool sequence was Bash 22, `read_capabilities` 11, `read_file` 3,
`secret_store` 2, and one each of `web_fetch`, nonexistent `read_models`,
`read_harnesses`, and `grep_files`. Every generation used Anthropic
`claude-opus-5`; hosted tool search was enabled at threshold 15. The trace's
35-tool catalog never contained `read_models`.

The turn's facts reproduce the reported failure:

- sequence 445: `read_models` failed with `Tool definition not found:
  read_models`;
- sequences 669 and 680: the first `secret_store` used `key` and failed because
  `name` is required, then the retry succeeded;
- sequence 695: Bash networking failed because the `http_client` feature and
  URL allowlist were not configured;
- the final provider completion itself began with internal narration before the
  user-facing answer. This was one `stop` completion, not UI concatenation of a
  reason-phase message;
- the live model API returned model row
  `model_01933b5a000070008000000000000227`, `model_id = gpt-5.6-terra`, display
  name `GPT-5.6 Terra`, provider OpenAI.

No credential value is reproduced in this document.

## Executive diagnosis

Platform Chat was asked to provision one durable autonomous workflow, but its
model-visible management API can only create a partial agent. It cannot discover
models, set `default_model_id`, discover/install/attach MCP or plugins, bind a
credential to the future execution principal, or create an agent-owned trigger.
The model repeatedly searched a capability catalog that cannot contain those
operations, guessed `read_models`, and eventually stored the credential in the
wrong session before giving up.

Runtime controls amplified the product-surface failure. Literal loop detection
was present, but the newer semantic progress guard was not enabled. Anthropic
prompt-cache support existed but the harness did not opt in. Cumulative-cost
compaction existed, but durable provider compaction is unavailable on this
driver; masking bounded individual requests without avoiding 30 uncached prompt
replays. The default per-turn ceiling is 500 iterations, so 30 iterations was
not close to a hard stop.

Yolop solves adjacent parts with conversational model tools, always-on prompt
caching, a persistent progress guard, mutable MCP/extension configuration, and
out-of-band secret prompts. Its local, single-user trust model is materially
different. The MCP and secret mechanisms cannot be copied into a hosted,
multi-tenant control plane.

## Pipeline findings

### 1. Platform Chat prompt and planning

**Classification: genuinely missing workflow contract; no proven historical
regression.**

`crates/server/src/harnesses/platform_chat.rs::definition` inherits `generic`
and adds only `platform_management`. Its `SYSTEM_PROMPT` explains entity links,
running an existing agent, generic harness reuse, and creation confirmations.
It says nothing about model resolution, integration provisioning, credential
scope, agent triggers, progress checkpoints, or keeping internal narration out
of the final answer.

Yolop's `src/runtime/system.md` is shorter but has explicit workflow and output
contracts: tool schemas are authoritative, hidden schemas are loaded through
tool search, validation should be decisive, and output should lead with the
result while hiding internal tool names. This reduces guessing but does not
create absent tools.

Recommendation: add a Platform Chat-specific provisioning contract only after
the required tools exist. Add a trace/eval fixture for “create an hourly agent
using model X and MCP Y” and assert one clean final answer. Prompt changes alone
would merely fail earlier.

### 2. Reason/act catalog, tool search, and invalid calls

**Classification: catalog parity is working; unknown-tool repair is genuinely
missing; Bash disclosure regression already fixed on latest main.**

The incident's reason requests advertised the same registered tool names that
act could execute. `read_models` was absent, then act failed closed at
`crates/core/src/atoms/act.rs` with `Tool definition not found`. This is a model
hallucination against an incomplete surface, not evidence that reason advertised
a tool act lacked.

`crates/builtins/src/tool_search.rs` preserves the full registered
schema behind deferred disclosure. `ToolCallRepair` only repairs malformed
arguments; it does not map unknown names. Yolop latest has no host-level
unknown-tool alias or semantic repair either; it also relies on exact dispatch.
Its advantage is that `src/capabilities/host.rs::SearchModelsTool` actually
exists and is named in `MODELS_PROMPT`.

There is one real schema/execution parity bug inside the registered management
surface: `ManageAgentsTool::parameters_schema` advertises `capabilities` for the
shared create/update tool, but `manage_agents_impl` reads that field only in the
`create` branch. The `update` branch and `PlatformStore::update_agent` silently
have no capability parameter. An agent therefore cannot attach an already
installed capability after creation even though the schema implies it can.

Everruns commit `5c0325bf` (`fix(tools): keep bash schema loaded (#2959)`) landed
after the incident. It makes Bash never deferrable and adds
`test_anthropic_opus5_hosted_search_calls_bash_contract`, addressing Claude's
observed tendency to invent `bash_run` or the wrong Bash arguments. Commit
`a33bf3d4` (`fix(chat): summarize repeated tool activity (#2960)`) also landed
afterward, but only improves activity narration; it does not stop semantic
loops.

Recommendation: retain exact fail-closed dispatch. Add a bounded “unknown tool;
available close matches are …” result and count it as no progress. Do not
silently alias mutating tools.

### 3. Model discovery and agent model selection

**Classification: provider search primitive already fixed; Platform Chat
surface is genuinely missing and has drifted behind the public agent API.**

The product model is already correct:

- `crates/core/src/agent.rs::Agent` owns `default_model_id`;
- `crates/server/src/domains/agents/types.rs::{CreateAgentRequest,
  UpdateAgentRequest}` and `crates/server/src/api/agents.rs` accept
  `default_model_id` and `mcpServers`;
- `knowledge/foundations/models.md` and
  `knowledge/integrations/model-router.md` define agent default and session
  override behavior.

The conversational adapter is not: `crates/core/src/capabilities/
platform_management.rs::{ReadAgentsTool, ManageAgentsTool}` omits model and MCP
fields, and `crates/core/src/platform_store.rs::PlatformStore::{create_agent,
update_agent}` cannot carry them. There is no models tool.

Commit `8dc932c1` (`feat(provider): search model catalogs across providers
(#2961)`) landed after the incident and added
`crates/provider/src/model_discovery.rs::{match_models,
search_provider_models}`. It did not wire Platform Chat or agent management.

Yolop commit `5b50b9b` (`feat(models): search provider catalogs
conversationally (#540)`) supplies the host pattern:
`src/capabilities/host.rs::{SearchModelsTool, SetModelTool}` calls
`src/capabilities/model_discovery.rs` and resolves before applying. Everruns
should reuse its own provider primitive, not Yolop host state.

### 4. MCP/plugin discovery, installation, attachment, and configuration

**Classification: backend already exists; Platform Chat surface genuinely
missing; hosted/local differences intentional.**

Everruns already has the control-plane primitives:

- org MCP CRUD and encrypted API keys in
  `crates/server/src/domains/mcp_servers/commands.rs`;
- decrypt-at-use and redacted reads in
  `crates/server/src/domains/mcp_servers/{service.rs,queries.rs}`;
- scoped remote MCP resolution and SSRF checks in
  `crates/server/src/domains/mcp_servers/scoped_mcp.rs` and
  `knowledge/integrations/mcp-servers.md`;
- plugin installation in `crates/server/src/domains/plugins/commands.rs`, with
  `mcpServers` compilation in `crates/core/src/plugins/compiler.rs`.

`PlatformManagementCapability::tools` exposes none of MCP-server search/CRUD,
plugin search/install, or attachment. `ManageAgentsTool` cannot set
`mcpServers`. Repeating `read_capabilities` could therefore never find Visti:
MCP servers and plugins are not capability rows.

Yolop `knowledge/specs/mcp.md` and `src/capabilities/mcp.rs` support global or
workspace `.mcp.json`, environment expansion, authenticated server upsert, and
live reload (commits `546df40` and `cfef0de`). That is appropriate for a local
user controlling their workspace and process. Hosted Everruns intentionally
supports remote HTTP MCP, validates outbound destinations, separates org
permissions, and audits mutation. `knowledge/security/threat-model.md`
TM-PLUGIN-001 through
TM-PLUGIN-008 document the supply-chain, OAuth-confusion, and SSRF boundaries.

Recommendation: expose read/search first. Keep plugin installation admin-gated
and confirmation-gated. Route every mutation through existing commands and the
session owner's caller; never add raw store writes or local stdio semantics.

### 5. Secret lifecycle and target binding

**Classification: session isolation and no model-visible plaintext transfer are
intentional; a safe target-binding workflow is genuinely missing.**

`secret_store` is session-scoped by contract
(`knowledge/execution/capabilities.md`,
SessionStorage). A credential written by Platform Chat cannot be read by a
future agent session. This isolation is correct. Although
`crates/server/src/domains/session_storage/commands.rs::BatchSetSessionSecrets`
can address an owned target session through the authenticated API, Platform
Chat has no such tool, and forwarding a value already present in model context
would be a weak provisioning design.

Org-managed MCP `api_key` is encrypted at rest, decrypted only during MCP use,
and never returned. Scoped `mcpServers.headers`, however, are literal;
`${ENV}` is intentionally not expanded. Persisting the Visti channel key there
would put plaintext into agent-authored configuration. Moreover, the Visti
contract observed in the incident passes `channel_key` as a `visti_send` tool
argument, not necessarily as standard bearer authentication. A generic MCP
header/API-key field is not automatically a compatible binding.

Yolop's safer interaction pattern is in commit `c32e644`:
`src/extensions/manage.rs::SetSecret` refuses an agent-supplied value and asks
the UI out of band; `src/extensions/secrets.rs::{Secret,ExtensionSecrets}`
redacts debug output and stores credentials separately; and
`src/extensions/capability.rs` injects the value host-to-extension while
stripping secret config. The test
`set_extension_secret_refuses_agent_value_and_needs_a_prompt` locks this down.

Recommendation: design a hosted equivalent that prompts the authenticated user
outside model context, stores an encrypted typed credential, binds it to the
specific MCP server plus agent (or execution identity), and injects it only on
the server-to-MCP call path. It needs an adapter for argument-style credentials
such as Visti or an explicit decision that such servers are unsupported. Do not
add a general cross-session secret-copy tool as the primary UX.

### 6. Agent versus session scheduling

**Classification: durable agent scheduling already exists; conversational
management is genuinely missing; Yolop scheduling is intentionally different.**

The correct Everruns object is an agent-owned trigger. The contract is explicit
in `crates/server/src/domains/agent_triggers/{mod.rs,commands.rs}`: a durable
cron schedule wakes the agent on its own harness. `CreateAgentTrigger` and the
routes in `crates/server/src/api/agent_triggers.rs` already support it, with cron
validation, limits, policy enforcement, and run history tests in
`domains/agent_triggers/tests.rs`.

The inherited `create_schedule` tool in
`crates/core/src/capabilities/session_schedule.rs` schedules a task in the
current session. Used by Platform Chat, it would wake Platform Chat—not the new
dad-joke agent. Platform management exposes no agent-trigger CRUD.

Yolop's `knowledge/specs/background.md` schedules local session wakes and
monitors. It does not model a reusable hosted agent plus durable control-plane
trigger. That mechanism should not be ported.

### 7. Loops, progress accounting, and budgets

**Classification: semantic guard implementation already ported but not enabled;
literal loop detector is intentionally narrow; completion gate exists but is
not wired to this host; per-turn budget is too permissive for Platform Chat.**

`crates/server/src/harnesses/generic.rs::definition` enables
`loop_detection`, which catches identical batches/results/read ranges. Eleven
similar searches with changing arguments/results need semantic state, so this
incident stayed outside its contract.

Commit `fa3da598` (`feat(capabilities): progress guard (#2930)`) ported Yolop's
semantic guard into `crates/builtins/src/progress_guard.rs`. It detects
repeated exploration and unchanged observations, warns at 24 tool calls, and
requests a checkpoint at 48. `knowledge/execution/capabilities.md`
intentionally makes it
opt-in, and Generic/Platform Chat do not enable it. Yolop latest registers
`ProgressGuardCapability::open` in `src/runtime/mod.rs`; its history includes
`639fcbc`, `a606aec`, `ea3b82c`, and `ada7967`.

Everruns commit `eb3e1273` ported Yolop's cheap completion classifier and
continuation budget to `crates/core/src/turn_completion.rs` (from Yolop commit
`c54c85a`). Yolop latest commit `ab206f5` now removes its duplicate
implementation: `src/session_state/task_completion.rs` re-exports Everruns'
`CompletionState`, `GateDecision`, and `ContinuationBudget` while retaining
host policy. A repository-wide Everruns call-site search finds only the core
unit tests: no Everruns host calls `gate_turn` or `ContinuationBudget` yet. In
any case it is an inter-turn continuation gate, not an in-turn semantic-loop
breaker.

`crates/core/src/runtime_agent.rs::default_max_iterations` is 500. The incident
ended voluntarily at 30, not because a meaningful Platform Chat budget fired.
`knowledge/security/threat-model.md` TM-AGENT-007 still marks the lack of a
per-response tool
call cap open.

Recommendation: enable progress guard for Platform Chat first, with trace tests
covering repeated catalog reads and unknown tools. Then set a Platform Chat
iteration/tool budget and emit a structured “blocked with missing capability”
checkpoint before cost balloons. Do not globally lower Generic's budget without
long-running-agent data.

### 8. Context growth, caching, compaction, and cost

**Classification: Anthropic cache support and cumulative-cost compaction already
exist; harness cache activation is genuinely missing; provider-native compaction
difference is intentional.**

Everruns has `crates/builtins/src/prompt_caching.rs` and Anthropic
`cache_control` support/tests in `crates/anthropic/src/driver.rs`. Generic does
not include `prompt_caching`, so every incident generation recorded zero cache
reads and creations. Yolop always registers `PromptCachingCapability::new()` in
`src/runtime/mod.rs`.

Commit `92fe74ea` (`feat(compaction): trigger checkpoints from cumulative cost
(#2933)`) already added uncached cumulative-cost thresholds in
`crates/builtins/src/compaction.rs` and the reason atom. It corresponds
to Yolop commit `54fe36d`. The incident trace shows no context-compaction event.
The reason path only requests durable proactive compaction when the driver
supports compact; Anthropic does not expose that provider-native operation.
Older results were masked, bounding each individual prompt, but the growing
conversation was still replayed uncached 30 times.

Recommendation: enable `prompt_caching` on Generic/Platform Chat after an
Anthropic trace regression test proves cache creation then cache reads across a
tool loop. Keep cumulative-cost masking/compaction as a backstop, not a
substitute. Add per-turn cumulative input/cost soft limits to Platform Chat.

### 9. Final-answer separation

**Classification: prompt/eval gap, not a reason/act transport regression.**

The final narration and answer were emitted in one provider completion and
stored as one final message. Everruns correctly separated phases; the model did
not honor an absent output contract. Yolop's `src/runtime/system.md` explicitly
constrains final output, and its continuation prompt asks for exactly one final
answer. Neither codebase has a reliable semantic sanitizer for prose like “let
me do a final check.”

Recommendation: add an explicit Platform Chat output contract and regression
eval. Avoid brittle prefix stripping, which can delete legitimate answers.

### 10. Authorization constraints on any port

**Classification: intentionally different and security-critical.**

Platform management is model-triggered org mutation.
`knowledge/harnesses/harness-types.md` and
`knowledge/security/threat-model.md` TM-AGENT-017 require execution as the owning
session's real caller, through the active permission resolver and command
boundary. TM-AUTHZ-013 separates MCP/plugin permissions; plugin management
remains admin-only. MCP/plugin installation additionally crosses external
fetch, SSRF, OAuth-token selection, and prompt-injection boundaries.

Yolop is a local, single-user application that may treat workspace-file edits
and process environment as user consent. Everruns must preserve org scope,
auditing, explicit confirmation, encrypted storage, redacted reads, and remote
network policy. A direct port of `.mcp.json`, `${ENV}`, local stdio, or extension
connection files would be unsafe.

## Classification summary

| Finding | Classification on latest Everruns main |
|---|---|
| Bash schema lost behind hosted search | Already fixed by `5c0325bf` |
| Repeated activity overwhelms chat UI | Already fixed cosmetically by `a33bf3d4` |
| Provider-neutral model catalog matching | Already fixed at library layer by `8dc932c1` |
| Cumulative uncached-cost compaction trigger | Already fixed by `92fe74ea` |
| Semantic progress guard implementation | Already ported by `fa3da598`; intentionally opt-in, missing on Platform Chat |
| Model search plus `default_model_id` in Platform Chat | Domain command exists; generic Platform command adapter is genuinely missing |
| MCP/plugin search, install, and agent attachment | Domain commands exist; generic Platform command adapter is genuinely missing |
| Secure out-of-band target credential binding | Genuinely missing |
| Agent-trigger management from Platform Chat | Domain command exists; generic Platform command adapter is genuinely missing |
| Prompt caching in Generic/Platform Chat | Genuinely missing activation |
| Unknown-tool suggestions/semantic no-progress accounting | Genuinely missing; exact failure is intentional |
| `manage_agents(update)` advertises but ignores `capabilities` | Regression / schema-execution drift |
| Session-scoped secrets; GET/HEAD web fetch; Bash allowlist; hosted remote-only MCP | Intentionally different security boundaries |
| Platform Chat narration prefix | Missing prompt/eval coverage; no evidence of phase regression |
| Completion gate implementation without host integration | Port landed, integration genuinely missing |

## Prioritized port matrix and PR boundaries

| Priority / PR | Scope | Expected impact | Risk | Dependencies | Required tests |
|---|---|---|---|---|---|
| P0-A | Extract MCP Tier-2 catalog/search/script execution into one transport-neutral command surface; leave MCP behavior unchanged | Gives MCP and Platform one authoritative `discover/query/execute` implementation | Medium: broad command inventory and error-shape compatibility | Existing MCP endpoint catalog, Bashkit, and `Command::run` | MCP before/after snapshots; catalog parity; read-only exclusions; positional rewriting; limits; sanitized errors |
| P0-B | Add a Platform runtime service bound server-side to the owning session, org, and effective human caller | Makes the shared command surface usable from workers without an internal-caller bypass | High: authorization and cross-org isolation | P0-A; direct and distributed runtime adapters | default/custom policy; member/admin matrix; cross-org denial; caller reconstruction; RPC tampering |
| P0-C | Register built-in `platform` capability with shared `discover/query/execute`; attach it to Platform Chat and retain `platform_management` temporarily | Solves model, MCP, Agent, plugin, and Trigger drift without bespoke CRUD tools | Medium: high-impact generic mutation surface | P0-A and P0-B | tool-definition parity; Terra/model selection; MCP + Agent + Trigger round trip; partial failure; prompt eval |
| P0-D (design first) | Typed, encrypted, out-of-band integration credential binding to MCP server + Agent/execution identity; define argument-style credential adapters | Makes Visti usable without exposing or marooning its key | High: secret disclosure and confused deputy | Product/UI prompt seam; MCP credential contract; audit model | value never enters transcript/tool args/events; redacted reads; wrong-Agent denial; rotation/revocation; Visti adapter integration |
| P1-A | Enable `progress_guard` for Platform Chat; add a Platform-specific iteration/tool/cost ceiling | Stops repeated catalog/query exploration and fails with a useful blocker | Low–medium: false positives on long tasks | None | replay-derived repeated-read loop; similar-but-progressing reads; unknown tool; checkpoint finalization |
| P1-B | Enable `prompt_caching` for Generic/Platform Chat | Large Anthropic input-cost reduction; incident had 510k uncached input tokens | Low–medium: provider semantics/cost accounting | Existing Anthropic driver support | two-generation Anthropic cache trace; stable breakpoint; usage/cost accounting; unsupported-provider no-op |
| P1-C | Platform command-use/output prompt plus end-to-end Visti eval | Uses `discover/query/execute` in dependency order and emits one clean final answer | Low, but ineffective before P0-C/P0-D | P0-C and P0-D | hourly Visti+Terra happy path; missing credential; denied command; partial mutation; no narration prefix |
| P2-A | Unknown-tool error with bounded close matches; count unchanged calls as no progress | Faster self-correction without unsafe aliases | Low | Progress accounting useful but optional | typo/no-match; mutating names never auto-substituted; registered deferred tool still executes |
| P2-B | Integrate `gate_turn`/continuation budget in the relevant Everruns hosts | Avoids premature tool-only completion in other workflows | Medium: extra turns/cost | Host-specific semantic evaluator decision | achieved/blocked/in-progress; 6-turn/64k/10m limits; no duplicate mutations |

P0-A through P0-C form one staged feature but remain separate reviewable PRs:
refactor without behavior change, establish the authorization seam, then expose
the capability. P0-D remains an independent security design and PR because no
generic command surface should carry a plaintext credential through model
context. Runtime controls P1-A/P1-B remain separate from control-plane access.

## Focused validation performed

- Exported and filtered the exact 772-event session trace; verified turn
  timestamps, tool counts/order, failures, provider request options, cache
  usage, and single-completion final narration.
- Queried the running `/api/v1/models` catalog and matched
  `gpt-5.6-terra` exactly.
- Compared every incident tool name against the generation catalog; only the
  attempted `read_models` was unregistered.
- Read the current tool schemas and store traits; verified
  `ManageAgentsTool`/`PlatformStore` cannot carry `default_model_id` or
  `mcpServers` while the public agent domain can.
- Searched all Everruns Rust call sites for `gate_turn` and
  `ContinuationBudget`; only unit tests currently use them.
- Ran focused `everruns-core` unit tests: all 24 progress-guard tests, all 5
  prompt-caching tests, and all 11 turn-completion tests passed. This validates
  the existing primitives, not their activation in Platform Chat.

## Decision

Do not implement or ship a mixed fix from this investigation. The minimum true
end-to-end solution spans at least model/agent management, durable triggers,
MCP/plugin management, and a new hosted credential-binding design. Combining
those would obscure authorization review and make rollback unsafe.

Prefer one generic `platform` capability backed by the same command inventory as
Everruns MCP over separate model/MCP/Agent/Trigger adapters. Its
`discover/query/execute` tools expose the existing domain commands while
retaining `Command::run` authorization. Agent-owned credential binding remains
a separate security primitive. Progress guarding and prompt caching can
proceed independently as cost/runaway hardening, but neither should be
represented as fixing the autonomous Visti workflow.
