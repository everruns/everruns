# Leased Resources

Leased resources are the generic control-plane primitive for any provider-owned state that must be cleaned up eventually. Daytona sandboxes and Browserless persistent sessions are the first adopters, but the model is intentionally cross-provider: any subsystem that creates remote state with a lifecycle beyond a single tool call should register a lease instead of inventing another ad hoc cleanup path.

## Intent

- Provide one persistent cleanup contract for remote resources created by tools or background workflows.
- Survive server and worker restarts in full mode by storing lease state in the control-plane database and driving cleanup from the durable scheduler.
- Expose a shared observability surface: durable schedule definition, schedule executions, worker task history, structured cleanup logs, and a session Resources UI/API.
- Keep provider-specific logic limited to cleanup handlers. Lease registration, retry, claiming, visibility, and lifecycle state stay generic.

## Model

See [`crates/core/src/leased_resource.rs`](/Users/mykhailochalyi/.codex/worktrees/baaf/everruns/crates/core/src/leased_resource.rs) for the public domain type and [`crates/server/migrations/008_v0.8.6.sql`](/Users/mykhailochalyi/.codex/worktrees/baaf/everruns/crates/server/migrations/008_v0.8.6.sql) for persistence details.

Important constraints:

- A lease stores an absolute `lease_expires_at`, not only `last_touched_at`, so cleanup queries stay index-friendly.
- Identity is `org + provider + resource_type + external_id`. Repeated tool activity must refresh the same row, not create duplicates.
- `session_id` is optional at the model level because cleanup may continue after the session row is deleted.
- `owner_user_id` records which user connection created the resource so cleanup reuses the same provider identity when possible.
- `metadata` is non-secret only. It exists for UI/debugging and cleanup handler inputs, not for bearer tokens.

## Lifecycle

Tool-side integrations use the generic store in [`crates/core/src/traits.rs`](/Users/mykhailochalyi/.codex/worktrees/baaf/everruns/crates/core/src/traits.rs):

- `upsert_resource`: create or extend a lease after successful remote creation or use.
- `release_resource`: fast-path explicit deletion/close operations.
- `list_resources`: session visibility for API/UI.

Control-plane cleanup uses backend-only transitions:

- Claim due rows where `lease_expires_at <= now()` or where a previous `cleaning` claim is stale.
- Move the row to `cleaning` and stamp `cleanup_started_at`.
- On success, mark `released`.
- On failure, move to `cleanup_failed`, store the error, and push `lease_expires_at` forward for retry.
- Success and failure writes are compare-and-set on `cleanup_started_at` so stale workers cannot overwrite a refreshed lease or newer claim.

## Scheduling And Observability

Cleanup is driven by the durable scheduler, not session schedules. The bootstrap lives in [`crates/server/src/leased_resource_scheduler.rs`](/Users/mykhailochalyi/.codex/worktrees/baaf/everruns/crates/server/src/leased_resource_scheduler.rs) and creates a single activity schedule named `leased-resource-cleanup`.

Observability comes from three places:

- Durable scheduler state and execution history show whether the cleanup loop is being triggered.
- The cleanup activity returns a structured summary with claimed, released, failed, and skipped counts plus per-resource outcomes.
- Structured logs annotate resource id, provider, type, external id, and failure text for cleanup attempts.

## Runtime Modes

- Full mode: leased resources and the durable schedule survive restarts because both persist in PostgreSQL.
- Dev mode: the same leased-resource APIs, worker activity, and scheduler logic run against the in-memory backends, so behavior is functionally equivalent while the process is alive. Dev mode remains restart-ephemeral because the repo’s existing dev storage is intentionally in-memory.
- External workers: the same contract is available over gRPC, so remote workers can register leases during tool execution and run cleanup activity without direct database access.

## Provider Integration Contract

Any system that requires cleanup should follow this pattern:

1. Register or refresh a lease immediately after successful remote creation or use.
2. Record enough non-secret metadata for cleanup and UI inspection.
3. Resolve and persist the creating connection owner when provider cleanup depends on user identity.
4. Release the lease on explicit user-driven deletion when that is faster than waiting for expiry.
5. Add a cleanup handler keyed by `(provider, resource_type)` if the provider cannot be cleaned with existing handlers.

This keeps lifecycle management consistent even as new resource-owning capabilities are added.
