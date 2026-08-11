---
type: Specification
title: "Lua Execution Capability (experimental)"
description: "Experimental Lua execution capability (sandboxed VFS scripting; aims to supersede bashkit_shell)."
tags:
  - everruns
  - execution
---
# Lua Execution Capability (experimental)

> **Status: EXPERIMENTAL — Phase 1 skeleton.** Implemented by the opt-in
> `everruns-integrations-lua` crate, selected by the Framework/host `lua`
> feature, and gated by the `FEATURE_LUA` internal feature flag at registry
> build time. Not registered in production grades yet.

## Why

`bashkit_shell` (see `knowledge/execution/bashkit-requirements.md`) gives agents scripted
execution over the session workspace. It is excellent at shell-idiom file
munging but weak at structured-data work: everything is text, piping is
fragile, quoting is a footgun, and extending it with typed host functions
(HTTP, MCP, tool-calling) means teaching an emulated POSIX shell new builtins.

The `lua` capability provides a single sandboxed interpreter we fully own. It
targets, in order:

1. **Common logic** — control flow, conditionals (native Lua).
2. **Math** — `math` stdlib (native Lua).
3. **Virtual filesystem** — read / write / grep / traverse via a host `fs`
   table backed by the session `SessionFileSystem`.
4. **Data processing** — `json` host module (serde bridge); later csv/yaml/base64.
5. **HTTP** *(later)* — a host `http` module behind the egress allow-list.
6. **Functions** *(later)* — user-defined Lua libraries loaded from the VFS via
   a controlled loader (not the real `package`/`require`).
7. **Code mode** *(later)* — the agent's available tools registered as Lua
   functions, so one script orchestrates many tool calls per turn.

### Goal: supersede `bashkit_shell`

The intent is for `lua` to become the primary execution capability and for
`bashkit_shell` to be deprecated once `lua` reaches feature parity for the
workflows bash is used for today. Until then the two ship side by side and are
evaluated head to head (round-trips per task, token cost, success rate, sandbox
incidents). No bash removal happens before that evidence exists. See
"Migration" below.

## Architecture

Mirrors `bashkit_shell` so the proven scaffolding is reused:

- `LuaCapability` — `Capability` impl. `risk_level() = High`, admin-gated
  exactly like `bashkit_shell` (`check_high_risk_caps` /
  `require_admin_for_high_risk`). Depends on `session_file_system`; contributes
  the `file_system` feature.
- `LuaTool` — single `lua` tool. `cpu_bound`, `concurrency_class =
  "session_workspace"` (serialize concurrent workspace mutators in a batch),
  `persist_output`, `long_running`.
- `LuaVfs` — wraps `(SessionId, Arc<dyn SessionFileSystem>)`. Owns the
  `/workspace` ↔ session-store path translation (identical rules to
  `SessionFileSystemAdapter`: absolute, forward-slash, `/workspace` stripped,
  traversal/outside-workspace rejected). This is what makes tenant isolation
  free — every path resolves through the already session-scoped store.
- **Engine** — `LuaLimits` (data) + `engine::run(...)` in the opt-in integration
  crate. Builds that do not select the integration pull in no interpreter.

### Runtime choice

**Decision: `mlua` (vendored Lua 5.4, never LuaJIT) is the sole engine.** It is
actively maintained, ships the complete Lua 5.4 stdlib, and exposes the
memory/instruction controls the sandbox needs. `piccolo` (pure-Rust) was
prototyped behind the same seam for its no-C appeal, but rejected: effectively
unmaintained (no release since 2024-06) and a thin stdlib that would force us to
reimplement ~19 functions plus a Lua-pattern engine on a dead base — and the eval
below measured it failing tasks mlua passes. The pure-Rust safety win is moot
when the dependency gets no security fixes. The engine seam was kept minimal
(one `engine::run`); there is no longer a second engine.

The mlua trade-off vs piccolo is that the sandbox is **in-process native code**,
not memory-isolated like wasm/process boundaries. That is accepted for an
admin-gated experimental capability; see TM-LUA in `knowledge/security/threat-model.md` for
the residual and the out-of-process path if hostile-CPU isolation is needed.

## Sandbox model (multitenant safety)

One **fresh VM per invocation**, never shared across sessions or tenants. No VM
state outlives a single tool call. All controls are on by default — **no
configuration knobs**. Because mlua loads the full stdlib, the dangerous surface
is **scrubbed** rather than absent:

| Control | Mechanism |
|---|---|
| Stdlib loaded | Only `string`, `table`, `math`, `os`, `utf8` (`io`/`package`/`debug` never loaded). |
| Scrub dangerous globals | `io`, `package`, `require`, `load`, `loadstring`, `dofile`, `loadfile`, `collectgarbage` → `nil`; `os.execute/getenv/exit/remove/rename/tmpname/setlocale` → `nil`; `string.dump` → `nil`. Safe `os.time/date/clock` kept. |
| No native escape | Lua 5.4 (no FFI); `package`/`require` scrubbed so `package.loadlib` cannot `dlopen`; `debug` not loaded. |
| Memory cap | `Lua::set_memory_limit` (32 MiB); over-budget alloc → Lua error. Host-side reads bounded by `SessionFileSystem` quotas. |
| CPU / wall-clock | Instruction-count hook (every 100k ops) enforces an instruction budget + wall-clock deadline; outer `tokio::time::timeout` backstop. |
| Runtime containment | The VM runs on a `spawn_blocking` thread; `fs.*` calls marshal to the runtime over a channel. A pathological *synchronous* op (e.g. catastrophic Lua pattern in C, which the hook cannot interrupt) occupies one blocking-pool thread instead of stalling a runtime worker. Residual: not force-killable in-process (out-of-process is the robust fix). |
| Egress | No raw sockets. `http.*` is fail-closed: routed only through the host `EgressService` and requires a non-empty allow-list that permits the URL, else it is not defined. |
| Code mode | `tools.<name>` exposes only Auto/non-destructive/non-execution sibling tools; child context drops the registry (no recursion). |
| Output size | `print` capture capped at 64 KiB in-engine; result further shaped via `tool_output_sanitizer` / `ExecToolResultPayload`. |

The only way out of the VM is the injected host tables, all of which route
through session-scoped stores.

## Host API surface (implemented)

```lua
-- fs: backed by SessionFileSystem, /workspace-rooted
fs.read(path)            -> string                 -- text (lossy for binary)
fs.write(path, content)                            -- create/overwrite
fs.append(path, content)
fs.exists(path)          -> boolean
fs.stat(path)            -> { name, is_dir, size } | nil
fs.list(path)            -> { {name, is_dir, size}, ... }
fs.remove(path[, recursive])
fs.mkdir(path)
fs.grep(pattern[, path]) -> { {path, line_number, line}, ... }  -- indexed grep_files

-- data processing
json.decode(s)           -> value
json.encode(value)       -> string
base64.encode(s) / base64.decode(s)

-- when enabled by the environment (else the global is nil):
http.get(url)            -> { status, body }   -- allow-listed hosts only
http.post(url, body)     -> { status, body }
tools.<name>(args_table) -> result             -- call a sibling tool (code mode)

print(...)               -- captured, streamed as tool.output.delta, returned as stdout
return value             -- serialized back to the model (json-encodable)
```

The full Lua 5.4 stdlib (`string.*` incl. `format`/`find`/`match`/`gsub`,
`table.*` incl. `sort`, `math.*`, `os.time`/`os.date`) is available — no shims
needed. Open host modules (future): `csv`/`yaml`.

## Threat model (TM-LUA-\*)

The authoritative table lives in `knowledge/security/threat-model.md` (§15A), covering
TM-LUA-001..009: arbitrary code execution, CPU/time, memory, filesystem/tenant
isolation, network egress/SSRF (fail-closed `http.*`), dynamic-code/bytecode
loading, native/FFI escape, output-channel abuse, and code-mode tool re-entry.
The one stated residual is TM-LUA-002: in-process timeout is best-effort against
pathological synchronous C ops (out-of-process execution is the robust fix).

## Phased roadmap

- **Phase 1 — DONE.** Capability + `lua` tool, `fs.*` + `json.*`, sandbox limits,
  engine seam, the **`mlua` engine** (vendored Lua 5.4) with the async VFS bridge,
  full e2e tests. Feature-flagged (`FEATURE_LUA` + `lua` cargo feature), admin-gated.
- **Phase 2 — PARTIAL.** `print` capture+stream and TM-LUA threat-model coverage
  are done. The full Lua 5.4 stdlib (`table.*`, `string.*`, `math.*`, `os.*`) is
  provided directly by `mlua`, so the piccolo-era stdlib-shim gap (which had failed
  the sort task 3/3) no longer applies. **Open:** `os.*`-subset hardening and
  background-execution parity (`BackgroundExecutableTool`).
- **Phase 3 — PARTIAL.** `base64` module + bash-vs-lua eval harness
  (`research/lua-vs-bash`) done — see Evaluation results below. **Open:**
  `csv`/`yaml` modules.
- **Phase 4 — IMPLEMENTED (http + code mode); user libraries deferred.**

  - **`http.*` (target 5) — DONE.** `http.get(url)` / `http.post(url, body)`
    routed **only** through `ToolContext::egress_service` (the host egress
    boundary) and **fail-closed**: a request runs only if `network_access` has a
    non-empty allow-list that permits the URL; otherwise `http.*` is not even
    defined. No raw sockets; response bodies capped at 1 MiB. SSRF/exfil mitigated
    by the allow-list + the central egress boundary (TM-LUA-005).
  - **Code mode (target 7) — DONE.** The agent's sibling tools are registered as
    `tools.<name>(args) -> result`, dispatched over the same channel bridge as
    `fs.*`. Eligibility is decided by the shared `lua::is_code_mode_eligible`
    predicate (`gated_code_mode_tools` filters the live registry through it):
    only `Auto`-policy, non-destructive, non-`cpu_bound` tools; approval/client-side
    and the execution tools (`lua`/`bash`) are excluded. The child `ToolContext`
    drops `tool_registry`, so code mode cannot recurse (TM-LUA-009).
  - **Code-mode routing capability (`lua_code_mode`) — DONE.** Makes Lua the
    agent's *primary* action surface by hiding the code-mode-eligible tools from
    the model's direct tool list, so the agent must orchestrate them inside a
    `lua` script. See "Code-mode routing capability" below.
  - **User libraries (target 6) — deferred.** A controlled `require(path)`-style
    loader that reads a **text** Lua file from `/workspace` and returns its
    exports. Text-only loading stays within the script's trust level (no bytecode
    → TM-LUA-006 holds), but the loader must cap module count/recursion and
    resolve paths through `LuaVfs`. Not yet implemented.

## Code-mode routing capability (`lua_code_mode`)

A separate, composable capability (`integrations/lua/src/code_mode.rs`)
that turns code mode from an occasional optimization into the agent's default
action path. It exists to satisfy three constraints:

1. **Capability-extensibility only.** The single mechanism is the existing
   `ToolDefinitionHook` tool-filtering seam (`knowledge/execution/capabilities.md`). The hook
   runs after the runtime agent has merged its final tool list and drops every
   code-mode-eligible `ToolDefinition` before the schemas reach the model. No
   new agent-loop plumbing.
2. **Relies on the `lua` capability.** `lua_code_mode` declares `lua` as a hard
   dependency and reuses `lua::is_code_mode_eligible` as the *same* predicate the
   engine uses to expose `tools.<name>`. Because both sides share one predicate,
   the set hidden from the model is exactly the set Lua re-exposes — a tool can
   never become unreachable.
3. **Capabilities export their tools to Lua.** The export channel is the
   engine's `tools.<name>(args)` table; this capability is what makes that the
   agent's primary path rather than an opt-in.

Key property that makes this safe: the *executable* `ToolRegistry`
(`ToolContext::tool_registry`) is built from capability `tools()` independently
of the model-facing `ToolDefinition` list. The hook only edits the latter, so
hidden tools stay fully executable — the act atom passes the full registry into
the Lua child context, and code mode calls them directly (not back through the
model). Essential tools (the `lua`/`bash` execution tools, destructive,
approval-gated, client-side, `cpu_bound`) are never eligible and stay direct
tool calls.

**Discovery.** Hiding a tool removes its standalone schema, so the hook also
grafts a catalog of the hidden tools onto the **`lua` tool's description**, built
from the same `ToolDefinition` list it is filtering (the synthetic
`human_intent` argument is omitted). By default each entry is a typed signature —
`- name(a: number, b?: string) — first sentence` (required args first, optional
suffixed with `?`, types from the JSON Schema). The `full_schemas` config option
additionally embeds each tool's complete minified JSON Schema (`schema: {…}`) for
lossless discovery of nested/complex parameters, at a token cost. Either way the
policy sentence in the capability's system prompt tells the model *to* route
through Lua, and the catalog tells it *what is callable and with which
arguments*. Putting the catalog on the lua tool (not the always-on system
prompt) means it is paid only when `lua` is present.

- **Config:** `{ "keep_visible": ["tool_a", ...] }` force-keeps named tools as
  direct calls even when they would otherwise be routed through Lua. Default empty.
- **Risk:** `High` — inseparable from `lua` (scripted execution) and admin-gated
  like its dependency. Registered only when `FEATURE_LUA` is on, next to `lua`.
- **Evidence:** runnable end-to-end smoke test
  `crates/host/tests/lua_code_mode_test.rs` and the documented example
  `crates/host/examples/lua_code_mode_agent.rs` (both behind the host
  `lua` feature, run in CI) assert that the math tools are hidden from the model
  yet executed through one `lua` script.

## Migration (supersede `bashkit_shell`)

1. Reach parity for the file-munging workflows bash covers (Phase 2 eval gate).
2. Land code mode (Phase 4) — the capability bash cannot match.
3. Default new agents to `lua`; mark `bashkit_shell` deprecated in capability
   metadata; keep existing bash-assigned agents running.
4. Remove `bashkit_shell` only after a deprecation window with no parity gaps.

## Evaluation vs bash

| | `lua` | `bashkit_shell` |
|---|---|---|
| Structured data | Native tables + JSON | Text + fragile piping |
| Sandbox ownership | Fully ours, extensible host fns | bashkit builtin set |
| Path to HTTP / MCP / code-mode | First-class typed host functions | Awkward (emulated builtins) |
| Quoting / escaping footguns | None | Many |
| Model fluency | Good | Excellent (larger prior) |
| Compact shell idioms | Weaker | `grep | sed | awk` |
| Stdlib completeness | Full (mlua ships Lua 5.4 stdlib) | Full bashkit builtins |
| Status | Experimental | Shipped |

### Agent ergonomics (how well the model drives each)

A separate axis from raw capability: which engine the model emits *correct,
recoverable* calls in. Bash wins zero-shot fluency for throwaway text munging
(huge training prior, compact pipelines). Lua wins reliability for stateful,
structured, multi-step, or tool-orchestrating work: no shell-quoting footguns in
JSON tool args, the host API surface exactly matches reality (no "command not
found" dead ends), errors carry line numbers (tighter self-correction), and
return values come back as structured JSON. Net: complementary today; Lua is the
better substrate for code-mode (Phase 4), which is the workload that justifies
superseding bash.

### Evaluation harness

Lives in **`research/lua-vs-bash`** (a standalone crate over `everruns-host`,
excluded from the workspace) — **not** `test_cases/`, which is for manual UI
testing.

- **A/B design.** Identical agent/model/prompt-scaffolding; swap only the
  execution capability (`bashkit_shell` ↔ `lua`). N runs per task for variance.
- **Corpus, sliced by target** (logic/math, VFS munging, JSON/CSV transforms,
  multi-file edits, grep-and-summarize, report generation, code-mode). Each task
  = seeded workspace + goal + a deterministic Rust grader (LLM-judge, blinded to
  arm, for open-ended tasks).
- **Metrics** (most already emitted via `tool.started`/`tool.completed`, token
  usage, durations): task success; tool-call validity rate (malformed/`not
  found`/syntax); self-correction rate; round-trips; token cost + bytes returned;
  latency; sandbox incidents.
- **Decision gates** (wire to Migration above): parity on the munging slice
  before defaulting agents to Lua; a material win on structured/code-mode slices;
  no regression in sandbox incidents. The piccolo stdlib gap is expected to show
  up here and must be weighed.

The harness is implemented in `research/lua-vs-bash` (`lua-vs-bash` bin), runs
both arms over `everruns-host`, and grades against the resulting workspace.

### Empirical results

Real Claude (`claude-haiku-4-5-20251001`), 4 tasks × 3 runs/arm. Both Lua
engines were measured against the same bash baseline:

| arm | success | tool_calls | tool_errors | avg_iters | avg_ms |
|---|---|---|---|---|---|
| bashkit_shell | 12/12 | 18 | 0 | 2.5 | 2597 |
| lua — **mlua** | **12/12** | **12** | **0** | **2.0** | **2528** |
| lua — piccolo | 9/12 | 46 | 27 | 4.6 | 7218 |

Findings:

- **mlua matches bash on success and beats it on efficiency.** 12/12 with zero
  tool errors, ~33% fewer tool calls (12 vs 18) and fewer iterations — the
  structured-data edge materializes once the full stdlib is present (`json_sum`
  and `math`: one Lua call computes and returns the value; bash needs compute +
  write). This is the "replace bash" thesis confirmed with data.
- **piccolo's stdlib gap is a hard blocker.** `transform` (sort lines) failed
  3/3 on piccolo with 7 tool errors / 10 iterations each: the model reaches for
  `table.sort`/`table.insert`/`string.format`, which piccolo 0.3.3 does not
  define (its `table` lib exposes only `pack`/`unpack`). The same engine on mlua
  passed it in one call. `grep_count` showed the same cause at lower severity.
- **Decision.** Combined with piccolo being effectively unmaintained (no release
  since 2024-06 vs mlua's monthly cadence) and the ~19-function-plus-pattern-
  engine reimplementation cost, the evidence favors **mlua as the default
  engine**. piccolo stays behind the seam for anyone wanting a pure-Rust VM and
  willing to own the stdlib. License is a non-issue (both MIT; vendored Lua is
  MIT).

Re-run: `ANTHROPIC_API_KEY=… cargo run --manifest-path research/lua-vs-bash/Cargo.toml`
(optionally `EVAL_MODEL=…`, `EVAL_RUNS=…`).
