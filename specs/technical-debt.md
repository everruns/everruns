# Technical Debt Analysis

Analysis date: 2026-03-17. Covers all Rust crates (207,849 lines across 298 files) and UI (48,843 lines).

---

## Severity Legend

- **CRITICAL** — Actively causes bugs, perf issues, or blocks development
- **HIGH** — Significant maintenance burden, duplication, or design flaw
- **MEDIUM** — Shortcuts and hacks that work but accumulate friction
- **LOW** — Minor style/ergonomic issues

---

## 1. God Objects (CRITICAL)

### 1a. `memory.rs` — 6,463-line in-memory store

**File:** `crates/server/src/storage/memory.rs`

`InMemoryDatabase` manages 27 entity types through 196 methods and 21 `RwLock<HashMap>` fields. It is the single-file equivalent of the entire PostgreSQL storage layer (`repositories.rs`, 5,490 lines) but with no modularity.

**Symptoms:**
- 27 functional areas: Users, API Keys, Agents, Harnesses, Sessions, Events, LLM Providers, LLM Models, Capabilities, Files (with move/copy/grep), MCP Servers, Skills, Organizations, Connections, Apps, Schedules, Leased Resources, Notifications, etc.
- 4 identical `create_*_with_id()` idempotency blocks (agent, harness, provider — ~60 lines each, 95%+ identical)
- 40+ identical `org_id` filter expressions: `.filter(|a| a.org_id == org_id).cloned()`
- 3 identical search-tokenization list patterns (agents, harnesses, models)
- Agent/Harness method twins: `create_agent` ≈ `create_harness`, `update_agent` ≈ `update_harness`, etc. (95%+ identical)
- Nested lock ordering creates deadlock risk

**Cost:** Every new entity type requires modifying this 6K-line file. Every agent/harness behavior divergence requires updating two near-identical method pairs.

### 1b. `repositories.rs` — 5,490-line PostgreSQL store

**File:** `crates/server/src/storage/repositories.rs`

Same problem as `memory.rs` but for PostgreSQL. 203 methods, all entity types in one file.

### 1c. `grpc_service.rs` — 4,249-line gRPC handler

**File:** `crates/server/src/grpc_service.rs`

141 methods. Contains 4 verbatim copies of the 17-line Session-to-Proto struct construction (lines 825, 1075, 1127, 1178) despite `internal-protocol` already having `schema_session_to_proto()`. Message-to-proto conversion is copy-pasted 3 times.

---

## 2. Adapter/Wrapper Explosion (HIGH)

### 2a. 16 gRPC adapter wrappers — 2,708 lines of indirection

**File:** `crates/worker/src/grpc_adapters.rs`

16 separate structs (`GrpcAgentStore`, `GrpcHarnessStore`, `GrpcSessionStore`, `GrpcSessionFileStore`, `GrpcEventEmitter`, `GrpcImageResolver`, `GrpcMessageRetriever`, `GrpcSessionStorageStore`, `GrpcConnectionResolver`, `GrpcSessionMutator`, `GrpcLeasedResourceStore`, `GrpcScheduleStore`, `GrpcPlatformStore`, `GrpcSessionSqlDbStore`, etc.) that are thin wrappers around a single `GrpcClient`.

Each wrapper: holds `GrpcClient` + `org_id`, implements one trait, delegates to gRPC with manual proto conversion. Could be 3–4 consolidated types or a generic `GrpcAdapter<T>`.

### 2b. 10 generic adapter wrappers for org_id injection

**Files:** `crates/worker/src/worker_adapters.rs` (566 lines), `crates/server/src/direct_worker_adapters.rs` (2,707 lines)

10 `Adapter*<A: WorkerAdapters>` structs (`AdapterAgentStore`, `AdapterHarnessStore`, `AdapterSessionStore`, `AdapterLlmProviderStore`, `AdapterSessionFileStore`, `AdapterEventEmitter`, `AdapterImageResolver`, `AdapterMessageRetriever`, etc.) exist solely to prepend `org_id` to every call. A single `OrgScopedAdapter<A>` or passing org_id through context would eliminate all of them.

### 2c. Duplicate `ModelWithProvider` type

**Files:**
- `crates/core/src/traits.rs` (lines 87–97)
- `crates/worker/src/worker_adapters.rs` (lines 293–300)

Identical struct defined in two crates with manual field-by-field conversion at lines 441–461 in `worker_adapters.rs`.

---

## 3. LLM Driver Duplication (HIGH)

**Files:**
- `crates/anthropic/src/driver.rs` — 1,558 lines
- `crates/gemini/src/driver.rs` — 1,238 lines
- `crates/openai/src/driver.rs` — 346 lines (thin delegation)

Anthropic and Gemini share ~775 lines (24%) of duplicated logic:

| Pattern | Lines duplicated |
|---------|-----------------|
| Constructor (`new`, `with_base_url`, `from_env`) | ~160 |
| Data URL parsing (`data:image/jpeg;base64,...`) | ~28 |
| Empty text filtering | ~29 |
| Retry loop with exponential backoff | ~185 |
| Error detection (model-not-found, request-too-large) | ~120 |
| Model discovery | ~100 |
| Streaming state setup (Arc<Mutex> token counters) | ~100 |

**Hacks visible:**
- Gemini generates synthetic tool call IDs via counter (`call_0`, `call_1`) — will break on reordered events
- Hardcoded `image/jpeg` fallback for all HTTP image URLs in both drivers
- Magic number `1024` added to thinking budget without explanation (Anthropic line 367)
- Silent `[Audio content not supported]` placeholder text injected into conversations
- Gemini's `clean_schema()` recursively clones and strips `additionalProperties` without logging
- `response.text().await.unwrap_or_default()` in error paths silently discards error context

---

## 4. Store Boilerplate (HIGH)

### 4a. 9 identical `Db*Store` structs

**Files:** `crates/server/src/storage/{agent,harness,session,session_schedule,llm_provider,session_file,session_storage,leased_resource,message}_store.rs`

Each follows the exact same pattern:
```rust
pub struct DbFooStore { db: Database, org_id: i64 }
impl DbFooStore { pub fn new(db: Database, org_id: i64) -> Self { Self { db, org_id } } }
pub fn create_db_foo_store(db: Database, org_id: i64) -> DbFooStore { DbFooStore::new(db, org_id) }
```

Free function `create_db_*_store()` adds no value over `::new()`.

### 4b. 912+ `.map_err(|e| AgentLoopError::store(e.to_string()))?` occurrences

Spread across 54+ files. A `trait StoreResultExt` with a single `.store_err()?` method would eliminate all of them.

### 4c. 137 `serde_json::to_value(x).unwrap_or_default()` / `serde_json::from_value(x).unwrap_or_default()`

Repeated across 15+ storage and service files. Could be `fn json_val<T: Serialize>(t: &T) -> Value` and `fn from_json<T: DeserializeOwned + Default>(v: Value) -> T`.

### 4d. 20+ manual `row_to_domain()` functions

Each maps DB row fields to domain structs one-by-one. Could use `From` trait impls or a derive macro.

---

## 5. gRPC Layer Design (HIGH)

### 5a. Bolted-on batch optimization

`GetTurnContextRequest` exists because fetching agent + session + messages separately was too slow. The batch endpoint was added after the fact — individual getters came first.

### 5b. O(n) message lookup

`crates/worker/src/grpc_adapters.rs` line 355: `get_message()` loads ALL messages then does `.find()`. TODO admits a `get_message` RPC is missing.

### 5c. Three different error strategies

- `grpc_service.rs` uses `tonic::Status`
- `internal-protocol/lib.rs` uses `ConversionError`
- `grpc_adapters.rs` uses `AgentLoopError::ConnectionFailed`

No unified error mapping between them.

### 5d. 150MB gRPC message limit for images

Both `grpc_service.rs` (line 506) and `grpc_adapters.rs` (line 47) note this is a temporary workaround. Should use presigned URLs.

---

## 6. API Route Duplication (MEDIUM)

**53 API modules** in `crates/server/src/api/` each repeat:
1. `pub struct AppState { service, auth }` (unique per module)
2. `impl FromRef<AppState> for AuthState` (identical boilerplate, 53 copies)
3. `pub fn routes(state: AppState) -> Router` (same structure)
4. CRUD handler signatures with identical `Result<(StatusCode, Json<T>), (StatusCode, Json<ErrorResponse>)>` return types

The `FromRef` impl alone is 5 lines × 53 modules = 265 lines of pure copy-paste.

---

## 7. `#[allow(dead_code)]` Accumulation (MEDIUM)

**40+ instances** across the codebase. Categories:

| Category | Count | Example |
|----------|-------|---------|
| Future phase APIs (Phase 3 org features) | ~8 | `memory.rs:50` — organizations, org_members |
| Auth system (full OAuth, API key scoping) | ~15 | `auth/middleware.rs`, `auth/oauth.rs`, `auth/api_key.rs` |
| Deserialization-only fields | ~8 | `anthropic/driver.rs`, `slack_events.rs` |
| Capability/persistence planned features | ~5 | `durable/timeout.rs:143`, `core/feature_flags.rs:72` |
| Observation/telemetry | ~4 | `core/observation/otel.rs:41,53` |

Many of these are legitimate (deserialization fields, Phase 3 prep). The auth module has the highest concentration — suggests auth was built speculatively ahead of features that use it.

---

## 8. Specific Shortcuts and Hacks (MEDIUM)

### 8a. Open security vulnerability

`specs/threat-model.md` line 98: **TM-AUTH-007** — OAuth state parameter not validated in callback. Marked OPEN.

`crates/server/src/auth/routes.rs` line 770: Account linking flow TODO — not implemented.

### 8b. Circuit breaker integration incomplete

3 TODOs across `crates/durable/` and `crates/worker/` — circuit breaker code exists but is not wired to LLM calls or tool executions. The distributed circuit breaker (`crates/durable/src/reliability/distributed_circuit_breaker.rs` line 13) is essentially dead code.

### 8c. Temporary debug logging left in

`crates/server/src/services/event.rs` line 119: `// Log streaming/generation events at info level (temporary for debugging)` — still active.

### 8d. Worker heartbeat faked

`crates/worker/src/durable_worker.rs` line 314: `// TODO: track actual current_load in heartbeat` — heartbeat reports default load, not actual.

### 8e. Circuit breaker key not per-provider

`crates/worker/src/durable_worker.rs` line 1050: All LLM providers share one circuit breaker key. OpenAI outage trips Anthropic's breaker too.

### 8f. Workflow iteration count not tracked

`crates/worker/src/activities.rs` line 381: `// TODO: Track actual iterations when workflow supports it` — iteration metrics report defaults.

---

## 9. Large Files (MEDIUM)

Files over 2,000 lines (non-test):

| File | Lines | Issue |
|------|-------|-------|
| `storage/memory.rs` | 6,463 | God object (27 entity types) |
| `storage/repositories.rs` | 5,490 | God object (PostgreSQL mirror) |
| `grpc_service.rs` | 4,249 | 141 methods, massive proto conversion |
| `api/durable.rs` | 3,702 | Entire durable API in one module |
| `core/events.rs` | 3,217 | Event types + serialization + builders |
| `durable/persistence/postgres.rs` | 3,047 | All workflow persistence SQL |
| `api/slack_events.rs` | 2,974 | Slack integration in one file |
| `durable/persistence/memory.rs` | 2,968 | In-memory workflow store |
| `core/openresponses_protocol.rs` | 2,926 | OpenAI responses protocol impl |
| `worker/grpc_adapters.rs` | 2,708 | 16 adapter wrappers |
| `direct_worker_adapters.rs` | 2,707 | Direct DB adapter layer |
| `core/llm_model_profiles.rs` | 2,648 | Model metadata (may be data, not logic) |
| `core/capabilities/mod.rs` | 2,603 | Capability registry + resolution |
| `server/seed.rs` | 2,461 | Seed data |
| `core/observation/braintrust.rs` | 2,388 | Braintrust integration |
| `durable/engine/executor.rs` | 2,195 | Workflow executor |
| `core/capabilities/virtual_bash.rs` | 2,147 | Bash sandbox capability |
| `core/observation/otel.rs` | 2,115 | OpenTelemetry spans |
| `integrations/daytona/src/tools.rs` | 2,030 | Daytona tools |

---

## 10. UI Technical Debt (LOW)

| File | Lines | Issue |
|------|-------|-------|
| `lib/api/types.ts` | 1,897 | All API types in one file |
| `chat/chat-panel.tsx` | 1,035 | Complex chat component |
| `settings/providers/page.tsx` | 975 | Settings page monolith |
| `durable/queues/page.tsx` | 955 | Queue page monolith |

---

## Top 10 Refactoring Priorities

1. **Split `memory.rs` and `repositories.rs`** into per-entity store modules. Eliminates two 5K+ line god objects and makes agent/harness/session stores independently testable.

2. **Extract shared LLM driver base** — constructor, retry loop, data URL parsing, error detection, streaming state. Saves ~775 lines and prevents bug divergence between providers.

3. **Consolidate gRPC adapters** — Replace 16 thin wrappers with 3–4 grouped adapters or a generic `GrpcAdapter<T>`. Eliminate inline proto conversions that duplicate `internal-protocol` functions.

4. **Kill `Adapter*<A>` wrappers** — Replace 10 org_id injection wrappers with a single `OrgScope` passed through context. Eliminates ~1,200 lines.

5. **Extract `.map_err(|e| AgentLoopError::store(e.to_string()))?`** into a `StoreResultExt` trait. Touches 912+ call sites but is mechanical.

6. **Fix OAuth state validation** (TM-AUTH-007). Open vulnerability.

7. **Add `get_message` gRPC RPC** — Replace O(n) message lookup that loads entire conversation.

8. **Wire circuit breakers to LLM calls** — Code exists but is dead. Either integrate or delete.

9. **DRY the API module boilerplate** — Macro or trait for `FromRef<AppState>` + route builder pattern. Saves 265+ lines of pure copy-paste.

10. **Unify gRPC error handling** — Single error type or conversion layer between `Status`, `ConversionError`, and `AgentLoopError`.
