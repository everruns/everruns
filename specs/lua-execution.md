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

**Spike gate (do first).** Before committing to piccolo, validate two unknowns
with a short spike: (1) the async host-call bridge — a `fs.read` that round-trips
to an async `SessionFileSystem`; (2) stdlib/language coverage against real
model-authored snippets (`string.format`, `gmatch`, metatables, `pcall`). If
either fails badly, fall back to `mlua` via the seam.

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

## Host API surface (Phase 1)

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

-- json: serde bridge
json.decode(s)           -> value
json.encode(value)       -> string

print(...)               -- captured, streamed as tool.output.delta, returned as stdout
return value             -- serialized back to the model (json-encodable)
```

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

- **Phase 1** *(this skeleton)* — capability + `lua` tool, `fs.*` + `json.*`,
  sandbox limits as data, engine seam, path-translation + metadata tests.
  Feature-flagged, admin-gated, dev grades only.
- **Phase 2** — wire the `mlua` engine end to end, streaming `print`, background
  execution parity, more data modules (csv/yaml/base64), bash-vs-lua eval harness.
- **Phase 3** — `http.*` behind the egress allow-list.
- **Phase 4** — user libraries (controlled loader) + code mode (tools as Lua
  functions, per-tool policy enforced inside the script).

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
| Status | Experimental | Shipped |
