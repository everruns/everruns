# Lua Execution Capability (experimental)

> **Status: EXPERIMENTAL — Phase 1 skeleton.** Gated behind the `lua` cargo
> feature in `everruns-core` and the `FEATURE_LUA` internal feature flag at
> registry build time. Not registered in production grades yet.

## Why

`virtual_bash` (see `specs/bashkit-requirements.md`) gives agents scripted
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

### Goal: supersede `virtual_bash`

The intent is for `lua` to become the primary execution capability and for
`virtual_bash` to be deprecated once `lua` reaches feature parity for the
workflows bash is used for today. Until then the two ship side by side and are
evaluated head to head (round-trips per task, token cost, success rate, sandbox
incidents). No bash removal happens before that evidence exists. See
"Migration" below.

## Architecture

Mirrors `virtual_bash` so the proven scaffolding is reused:

- `LuaCapability` — `Capability` impl. `risk_level() = High`, admin-gated
  exactly like `virtual_bash` (`check_high_risk_caps` /
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
- **Engine** — `LuaLimits` (data) + `engine::run(...)`, behind the `lua` cargo
  feature; with it off, `engine::run` returns a "not compiled" error so the
  default workspace build pulls in no interpreter.

### Runtime choice

**Decision: `mlua` (vendored Lua 5.4, never LuaJIT) is the sole engine.** It is
actively maintained, ships the complete Lua 5.4 stdlib, and exposes the
memory/instruction controls the sandbox needs. `piccolo` (pure-Rust) was
prototyped behind the same seam for its no-C appeal, but rejected: effectively
unmaintained (no release since 2024-06) and a thin stdlib that would force us to
reimplement ~19 functions plus a Lua-pattern engine on a dead base — and the eval
below measured it failing tasks mlua passes. The pure-Rust safety win is moot
when the dependency gets no security fixes. The engine seam was kept minimal (one
`engine::run` + a not-compiled stub); there is no longer a second engine.

The mlua trade-off vs piccolo is that the sandbox is **in-process native code**,
not memory-isolated like wasm/process boundaries. That is accepted for an
admin-gated experimental capability; see TM-LUA in `specs/threat-model.md` for
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
| Egress | No network host functions and no socket library. HTTP (Phase 4) will be gated by the egress allow-list. |
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

print(...)               -- captured, streamed as tool.output.delta, returned as stdout
return value             -- serialized back to the model (json-encodable)
```

The full Lua 5.4 stdlib (`string.*` incl. `format`/`find`/`match`/`gsub`,
`table.*` incl. `sort`, `math.*`, `os.time`/`os.date`) is available — no shims
needed. Open host modules (future): `csv`/`yaml`.

## Threat model (TM-LUA-\*)

Parallels TM-BASH. Full integration into `specs/threat-model.md` is a Phase 1
follow-up; enumerated here for review:

- **TM-LUA-001 Arbitrary code execution.** Mitigated by the stdlib whitelist,
  global scrubbing, and High risk-level (admin-gated assignment).
- **TM-LUA-002 CPU exhaustion.** Instruction-count hook + deadline + outer
  timeout.
- **TM-LUA-003 Memory exhaustion.** `set_memory_limit` hard cap.
- **TM-LUA-004 Filesystem escape / cross-tenant access.** All paths resolve
  through `LuaVfs` → session-scoped `SessionFileSystem`; `/workspace`-rooted,
  traversal rejected. No `io` library.
- **TM-LUA-005 Network egress / exfiltration.** No network in Phase 1. Phase 3
  HTTP gated by the egress allow-list (`ToolContext::network_access`).
- **TM-LUA-006 Dynamic code / bytecode loading.** `load`/`loadstring`/`dofile`/
  `loadfile` removed; no untrusted bytecode path.
- **TM-LUA-007 Native escape (FFI).** Lua 5.4 only; LuaJIT forbidden.
- **TM-LUA-008 Output-channel abuse.** Sanitizer + hard output cap.

## Phased roadmap

- **Phase 1 — DONE.** Capability + `lua` tool, `fs.*` + `json.*`, sandbox limits,
  engine seam, **native-Rust piccolo engine** with the async VFS bridge, full
  e2e tests. Feature-flagged (`FEATURE_LUA` + `lua` cargo feature), admin-gated.
- **Phase 2 — PARTIAL.** `tonumber` shim; `print` capture+stream; TM-LUA in the
  threat model. **Open (P0, evidence-backed by the eval below):** stdlib shims —
  `table.sort/insert/remove/concat` and `string.format/find/match/gmatch/gsub/rep`
  (their absence fails the sort task 3/3). Then `os.*` subset and
  background-execution parity (`BackgroundExecutableTool`).
- **Phase 3 — PARTIAL.** `base64` module + bash-vs-lua eval harness
  (`crates/agent-evals`) done — see Evaluation results below. **Open:**
  `csv`/`yaml` modules.
- **Phase 4 — DESIGNED, deferred to dedicated security-reviewed PRs.** Each item
  expands the trust boundary and must clear `specs/threat-model.md` on its own:

  - **`http.*` (target 5).** `http.get/post(url, opts)` routed through
    `ToolContext::egress_service` and gated by `ToolContext::network_access`
    (the merged harness∩agent∩session allow-list). No raw sockets. New threats:
    SSRF to internal metadata endpoints, data exfiltration — this is the single
    biggest surface and the reason bash omits network entirely. Must extend
    TM-LUA-005 from "no network" to an allow-listed, audited egress.
  - **User libraries (target 6).** A controlled `require(path)`-style loader that
    reads a **text** Lua file from `/workspace`, compiles and runs it, and returns
    its exports. Text-only loading does not exceed the inline script's trust level
    (still no bytecode → TM-LUA-006 holds), but the loader must cap module count
    and recursion depth and resolve paths through `LuaVfs`.
  - **Code mode (target 7).** Register the agent's available tools as Lua
    functions (`tools.<name>(args) -> result`) dispatched over the same channel
    bridge as `fs.*`, so one script orchestrates many tool calls per turn. Gates:
    honor each tool's `ToolPolicy` (approval-gated tools cannot be silently
    called from a script), exclude other High-risk execution tools by default
    (no `lua` calling `virtual_bash`/agent-spawn unless explicitly allow-listed),
    bound total nested tool calls, and surface each call as a normal
    `tool.started`/`tool.completed` event for audit. This is the flagship
    "replace bash" capability and needs the most careful review.

## Migration (supersede `virtual_bash`)

1. Reach parity for the file-munging workflows bash covers (Phase 2 eval gate).
2. Land code mode (Phase 4) — the capability bash cannot match.
3. Default new agents to `lua`; mark `virtual_bash` deprecated in capability
   metadata; keep existing bash-assigned agents running.
4. Remove `virtual_bash` only after a deprecation window with no parity gaps.

## Evaluation vs bash

| | `lua` | `virtual_bash` |
|---|---|---|
| Structured data | Native tables + JSON | Text + fragile piping |
| Sandbox ownership | Fully ours, extensible host fns | bashkit builtin set |
| Path to HTTP / MCP / code-mode | First-class typed host functions | Awkward (emulated builtins) |
| Quoting / escaping footguns | None | Many |
| Model fluency | Good | Excellent (larger prior) |
| Compact shell idioms | Weaker | `grep | sed | awk` |
| Stdlib completeness | Partial (piccolo gap, see above) | Full bashkit builtins |
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

Lives in a dedicated **`evals/`** tree (or a `crates/agent-evals` crate over
`everruns-runtime`) — **not** `test_cases/`, which is for manual UI testing.

- **A/B design.** Identical agent/model/prompt-scaffolding; swap only the
  execution capability (`virtual_bash` ↔ `lua`). N runs per task for variance.
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

The harness is implemented in `crates/agent-evals` (`lua-vs-bash` bin), runs both
arms over `everruns-runtime`, and grades against the resulting workspace.

### Empirical results

Real Claude (`claude-haiku-4-5-20251001`), 4 tasks × 3 runs/arm. Both Lua
engines were measured against the same bash baseline:

| arm | success | tool_calls | tool_errors | avg_iters | avg_ms |
|---|---|---|---|---|---|
| virtual_bash | 12/12 | 18 | 0 | 2.5 | 2597 |
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

Re-run: `cargo run -p everruns-agent-evals --bin lua-vs-bash` (point the crate's
`everruns-core` feature at `lua-mlua` or `lua` to pick the engine).
