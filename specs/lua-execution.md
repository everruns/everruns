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
- **Engine seam** — `LuaLimits` (data) + `engine::run(...)`. The engine lives
  behind a cargo feature; with it off, `engine::run` returns a "not compiled"
  error so the default workspace build pulls in no interpreter. The capability,
  tool, VFS, host-API surface, and tests are engine-agnostic, so the chosen
  engine (`piccolo`, primary) and the reference engine (`mlua`, `lua-mlua`
  feature) are interchangeable behind this one module.

### Runtime choice

**Decision: native-Rust `piccolo`.** No C/FFI surface to audit, fuel-based
*true preemptive* CPU bounding, and sandbox-by-construction (the dangerous
libraries simply do not exist — there is nothing to scrub). Trade-offs accepted:
pre-1.0 maturity, a partial stdlib, and a manual async bridge for the VFS host
calls (piccolo has no `create_async_function` equivalent).

`mlua` (Lua 5.4, vendored) is retained only as a **reference engine** behind the
`lua-mlua` cargo feature — it proved the engine seam end to end and is a fallback
if the piccolo spike (below) shows it cannot run the scripts models actually
write. **Never LuaJIT** under any engine (FFI = instant escape).

**Spike gate (done).** Two unknowns were validated empirically:
1. **Async host-call bridge — PASS.** piccolo is synchronous, so the VM runs on
   a `spawn_blocking` thread and each `fs.*` callback marshals its request to the
   tokio runtime over an `mpsc` channel and `blocking_recv`s the reply. The async
   `SessionFileSystem` round-trips cleanly; fs/json round-trip tests pass.
2. **Stdlib/language coverage — PARTIAL (tracked debt).** piccolo 0.3.3 ships a
   thin stdlib. Confirmed missing and relevant to model-authored scripts:
   `tonumber` (base); `string.format`, `string.find`, `string.match`,
   `string.gmatch`, `string.gsub`, `string.rep` (string lib); no `os` library.
   `tonumber` is shimmed as a host function; the broader `string.*` gap is open
   Phase 2 work. This is real "replace-bash" debt: until these are shimmed (or
   piccolo gains them), some model-written Lua will fail where bash would not —
   the bash-vs-lua eval (Phase 3) must weight this. If the gap proves too costly,
   the `lua-mlua` engine (full Lua 5.4 stdlib) remains a seam-level fallback.

## Sandbox model (multitenant safety)

One **fresh VM per invocation**, never shared across sessions or tenants. No VM
state outlives a single tool call. Enforced at construction:

| Control | Mechanism |
|---|---|
| Stdlib whitelist | Load only `string`, `table`, `math`, `utf8`, and a safe `os` subset (`os.time`, `os.date`, `os.clock`). |
| No ambient I/O | `io`, `package`/`require`, `debug`, `os.execute/getenv/exit/remove/rename/tmpname` scrubbed to `nil`. |
| No dynamic code | `load`, `loadstring`, `dofile`, `loadfile` removed (bytecode loading is a classic escape vector). |
| No FFI | Lua 5.4, not LuaJIT. |
| Memory cap | `Lua::set_memory_limit` (default 32 MiB). |
| CPU / wall-clock | Instruction-count hook checks a deadline + max-instruction budget and aborts; outer `tokio::time::timeout` as backstop. |
| Egress | No network in Phase 1 (parallels TM-BASH-003). HTTP arrives in Phase 3 behind the egress allow-list. |
| Output size | Reuse `tool_output_sanitizer` + `ExecToolResultPayload` (16 KiB window, 64 KiB hard cap). |
| Runtime starvation | `cpu_bound` hint → ActAtom offloads to a dedicated task; instruction hook keeps a tight loop from pinning a worker thread. |

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
tonumber(s)              -- shimmed (piccolo base lacks it)

print(...)               -- captured, streamed as tool.output.delta, returned as stdout
return value             -- serialized back to the model (json-encodable)
```

Not yet available on the piccolo engine (Phase 2 stdlib work): `string.format`,
`string.find`/`match`/`gmatch`/`gsub`/`rep`, `os.*`, `csv`/`yaml` modules.

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
  threat model. **Open:** the `string.*` stdlib gap (format/find/match/gmatch/
  gsub/rep), `os.*` subset, background-execution parity (`BackgroundExecutableTool`).
- **Phase 3 — PARTIAL.** `base64` module done. **Open:** `csv`/`yaml` modules;
  the bash-vs-lua eval harness (see Evaluation below).
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
