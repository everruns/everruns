---
type: Specification
title: Control plane and data plane protocol
description: The HTTPS and JSON contract between the control plane and a data plane: config stream, org admin, feed, spend allowances and org tokens.
tags:
  - execution
  - data-plane-v2
  - protocol
---

# Control plane ↔ data plane protocol

Status: approved 2026-11-09, not implemented yet. Part of [design.md](design.md) (data plane v2).

## Shape

| Channel | Direction | Carries | Delivery |
| --- | --- | --- | --- |
| **Config** | control → data | Full agent definitions, tenant secrets and connections, tenant policy | Per-org versioned stream from an RDS outbox, retried until acknowledged |
| **Org admin** | control → data | Enable, suspend, resume, export, delete an org | Idempotent commands with a status to poll |
| **Feed** | data → control | Usage, session summaries, refreshed tokens, org events, health | Batched, at least once, deduplicated by cursor |
| **Keys** | data ← control | Public keys that verify org tokens | The data plane pulls JWKS and caches it |

**Transport:** HTTPS with JSON bodies, mTLS between the planes. This is the same stack as the public `/v1` API (axum, serde types, OpenAPI). Traffic is low volume (config on edits, feed batches about once a second), so nothing here needs streaming. Both sides serve an internal `/dp/v1` prefix that the edge router never exposes publicly.

**Session traffic is not part of this protocol.** Messages, SSE and webhooks go from the edge router straight to the data plane. If the control plane is down, running sessions continue; edits and new orgs wait until it is back.

**Placement is not part of this protocol either.** The control plane only says which data plane an org uses. Inside the data plane, any cell may host sessions of any org, and sessions move freely between cells (see "Placement" in [design.md](design.md)).

## Endpoints

Served by the data plane, called by the control plane:

| Method and path | Body → reply | Notes |
| --- | --- | --- |
| `PUT /dp/v1/orgs/{org}/config/{epoch}/{version}` | `ConfigUpdate` → `ConfigAck` | One call per tenant change set, in order; idempotent by `(org, epoch, version)` |
| `GET /dp/v1/orgs/{org}/config` | → `{ "epoch": u64, "applied_version": u64 }` | Resume the outbox after a restart, detect gaps |
| `GET /dp/v1/orgs/{org}/config/unknown?epoch={e}` | → `ConfigDiff` | After a control plane restore: agent revisions and secrets the data plane holds that the restored epoch doesn't know |
| `POST /dp/v1/feed/replay` | `{ source, incarnation, from_index }` → `CommandAck` | Ask a cell to resend feed records from the shipped log (30 days) |
| `PUT /dp/v1/orgs/{org}` | `OrgEnable` → `CommandAck` | Enable the org on this data plane: data key, placement policy |
| `POST /dp/v1/orgs/{org}/state` | `{ command_id, state: "active" \| "suspended", reason }` → `CommandAck` | Suspension takes effect immediately |
| `POST /dp/v1/orgs/{org}/exports` | `{ command_id, export_id }` → `CommandAck` | Writes `orgs/{org}/exports/{export_id}.tar.zst` |
| `DELETE /dp/v1/orgs/{org}` | `{ command_id }` → `CommandAck` | Tombstone now; bytes removed by the explicit deletion job |
| `GET /dp/v1/orgs/{org}/commands/{command_id}` | → `CommandStatus` | Progress of long commands |

Served by the control plane, called by the data plane:

| Method and path | Body → reply | Notes |
| --- | --- | --- |
| `POST /dp/v1/feed` | `FeedBatch` → `FeedAck` | At least once; deduplicated by `(source, incarnation, index, ordinal)` |
| `POST /dp/v1/tenants/{org}/allowances` | `AllowanceRequest` → `AllowanceGrant` | A cell asks for spend allowance when low and settles earlier grants; idempotent by `request_id` |
| `POST /dp/v1/connections/{id}/refresh-lease` | `{ holder, token_revision }` → `{ granted, expires_unix_ms }` | One refresher per connection token at a time (served by the data plane registry when `refresh_owner` is `DataPlane`; listed here for completeness) |
| `GET /.well-known/jwks.json` | → JWKS | Cached; refreshed on unknown `kid` |

## Types

```rust
// Shared crate, serde, published as OpenAPI. Unknown fields are ignored on read.

struct ConfigUpdate {
    epoch: u64,             // control plane epoch; a restored control plane starts a new one
    version: u64,           // monotonic per org within an epoch, assigned by the outbox
    base_version: u64,      // 0 = full snapshot; otherwise must equal the applied version
    changes: Vec<ConfigChange>,
}

#[serde(tag = "kind")]
enum ConfigChange {
    Agent(AgentDefinition),     // a new revision of one agent
    Secret(SealedSecret),       // tenant secret store (provider keys, connection tokens)
    Connection(Connection),
    Policy(TenantPolicy),
    Removal { of: String, id: String }, // running turns keep the revision they started with
}

/// A fully resolved agent: everything a turn needs. The data plane does not
/// know harnesses, org defaults or capability settings; the control plane
/// resolves them with the platform's existing code. An edit to a harness or
/// the org default model makes the control plane send new revisions of
/// every agent it affects. Stored in S3; announced on the change stream as
/// "agent A is now at revision R"; executors cache by (agent_id, revision);
/// a turn records the revision it ran with.
struct AgentDefinition {
    agent_id: String,
    revision: u64,          // monotonic per agent
    format: u32,            // agent package format version
    package: Value,         // instructions, model, provider, capabilities and settings,
                            // MCP servers, tools, members, egress rules;
                            // secrets only as SealedSecret ids
}

/// AES-256-GCM under the org data key. Cells store ciphertext only; executors
/// unwrap the data key through KMS and decrypt in memory.
struct SealedSecret { secret_id: String, revision: u64, ciphertext: Base64, nonce: Base64, data_key_id: String }

struct Connection {
    connection_id: String,
    kind: String,           // mcp_oauth | github | slack | ...
    settings: Value,
    token_secret_id: String,
    refresh_owner: RefreshOwner, // DataPlane | ControlPlane
}

struct TenantPolicy {   // the org, seen by the data plane only as a tenant
    limits: Limits,         // safety nets only: per-cell caps, file quotas
    egress: EgressPolicy,   // allowlists, caps, kill switch
    placement: PlacementPolicy, // Shared (default) | Dedicated { pool }
    features: Vec<String>,  // enabled feature stages
}

#[serde(tag = "result")]
enum ConfigAck { Ok { applied_version: u64 }, AlreadyApplied { applied_version: u64 }, NeedFullSnapshot { applied_version: u64 },
                StaleEpoch { epoch: u64 } }   // update from an older control plane epoch; refused

/// After a control plane restore: what the data plane holds that the restored epoch doesn't.
/// An operator re-imports or drops each entry before the new epoch's full snapshot applies.
struct ConfigDiff { agents: Vec<(String, u64)>, secrets: Vec<(String, u64)>, connections: Vec<String> }

struct OrgEnable { command_id: String, wrapped_data_key: Base64, data_key_id: String, policy: OrgPolicy }
struct CommandAck { command_id: String, accepted: bool, error: Option<String> }
struct CommandStatus { command_id: String, state: String, progress: Option<f32>, error: Option<String> }

struct FeedBatch {
    data_plane: String,
    source: String,         // the cell that produced the records
    incarnation: u64,       // bumped when the cell is rebuilt from a restore
    records: Vec<FeedRecord>, // ordered by (index, ordinal)
}

struct FeedRecord {
    index: u64,             // Raft index of the entry that produced it
    ordinal: u32,           // position among the records of that entry (one entry holds
                            // many batched commands, and a command can emit several records)
    org_id: String,
    at_unix_ms: i64,
    #[serde(flatten)] record: Record,
}

#[serde(tag = "type")]
enum Record {
    Usage { session_id: String, turn_id: String, attempt_id: String, // one record per provider
            // call attempt; a retry that bills again is a new attempt
            unresolved: bool, // process died before usage came back; reservation stays held
            agent_id: String, agent_hash: String,
            provider: String, model: String, input_tokens: u64, output_tokens: u64,
            cached_tokens: u64, cost_micros: i64, kind: String /* model | tool | sandbox | egress */ },
    Session { session_id: String, session_version: u64, // monotonic per session, carried across
                                                        // moves; an older version never overwrites a newer one
              agent_id: String, title: String,
              status: String /* idle | running | waiting | failed | deleted */,
              created_unix_ms: i64, last_activity_unix_ms: i64, turns: u64, cost_micros: i64 },
    SecretRefreshed { secret_id: String, revision: u64, ciphertext: Base64, nonce: Base64, data_key_id: String },
    ConfigApplied { version: u64 },
    OrgEvent { kind: String /* budget_exhausted | limit_hit | suspended */, detail: String },
    Health { sessions: u32, running_turns: u32, lag_ms: u32 },
}

struct FeedAck { source: String, incarnation: u64, stored_through: (u64, u32) } // (index, ordinal)

struct AllowanceRequest {
    request_id: String,
    cell: String,
    incarnation: u64,
    want_micros: i64,
    settlements: Vec<GrantSettlement>, // usage against earlier grants since the last request
}
struct GrantSettlement {
    grant_id: String,
    used_micros: i64,       // cumulative actual cost charged to this grant
    reserved_micros: i64,   // still reserved by operations in flight
    closed: bool,           // final: nothing more will be charged; the rest returns to the balance
}
struct AllowanceGrant {
    grant_id: String,       // reserved in the ledger when issued
    granted_micros: i64,    // 0 = budget exhausted or a policy stops spending
    spend_until_unix_ms: i64, // after this the cell stops spending from it; the money is
                            // released only by a closed settlement, never by time
    reason: Option<String>, // budget_exhausted | ask_to_continue | suspended
}
```

## Org tokens

The edge router and gateways check a JWT signed by the control plane (EdDSA). The keys come from `/.well-known/jwks.json`, which the data plane fetches and caches. No call goes back to the control plane per request.

| Claim | Meaning |
| --- | --- |
| `sub` | user id or API key id |
| `org` | org id |
| `dp` | data plane id (the edge routes on this) |
| `scope` | roles and scopes (`sessions:write`, `admin`, ...) |
| `exp` | short: about 10 minutes |
| `jti` | token id, for audit |

The browser and CLI get tokens from the control plane at sign-in and refresh them in the background. **API keys** are long-lived, so the edge exchanges a key for a short token once (`POST /v1/auth/token` on the control plane) and caches the result until it expires. Revoking a user or key therefore takes effect within the token lifetime. A suspended org is pushed through the org state endpoint, so suspension is immediate while the data plane is connected and takes at most 30 s through a partition (see Rules, freshness). A control plane outage longer than the token lifetime stops new sign-ins and token refreshes; running sessions continue.

## Rules

- **Ordering.**
  - Config versions apply strictly in order per org. A gap or mismatched base gets `NEED_FULL_SNAPSHOT`, and the outbox then sends a full snapshot.
  - A new control plane epoch (after an RDS restore) is accepted only as a full snapshot, after the operator has resolved the `ConfigDiff`. Updates from an older epoch get `STALE_EPOCH`.
  - Different orgs are independent.
- **Idempotency.**
  - Config by `(org, version)`.
  - Admin commands by `command_id`.
  - Feed records by `(source, incarnation, index, ordinal)` for transport; usage by `attempt_id` as its stable logical identity, so a replay or a migration can't bill the same call twice; session summaries by `session_version`.
  - Grants by `grant_id`; settlements are cumulative, so a repeated one changes nothing.
  - Every call can be retried safely.
- **Who is the source of truth.**
  - The control plane owns config, secrets and policy.
  - The data plane owns sessions, files, placement and running usage counters.
  - For a connection whose `refresh_owner` is `DATA_PLANE`, the newest token arrives through the feed, and the control plane stores it without refreshing on its own.
  - The control plane owns budgets: its ledger and usage policies decide, and cells enforce locally from spend allowances it grants. The guarantee is a strict cap on authorized spend: grants are reserved when issued, cells reserve each operation's maximum cost before starting it, and money returns to the balance only through a closed settlement. A lost cell's open grants count as spent until disaster recovery reconciles them. If the control plane is down, cells run until their allowances are spent, then new turns wait.
  - A refreshed connection token carries a higher revision than the one it replaces; a lower revision (delayed feed, old snapshot) never overwrites it.
- **Delivery and retention.** A cell keeps feed records until they are acknowledged, also while draining; a cell is retired only after its feed is fully acknowledged. The feed also stays in the shipped log for 30 days, so a restored control plane can replay from its own cursor.
- **Freshness.** Suspension and policy changes are pushed at once. Every executor, stream node and egress proxy must have heard from the change stream within 30 s, or it stops starting new work; that bounds suspension through a partition.
- **Evolution.** Additive changes only within `/dp/v1`: new fields, new tagged variants, new values, and readers ignore what they don't know. A breaking change means `/dp/v2`, with both served during the switch. These are the same rules as the session protocol.
- **Security.**
  - mTLS between the planes.
  - Plaintext secrets never cross: the control plane seals them, and only executors unseal.
  - Data keys are wrapped with the data plane's KMS key, so a dump of the cell doesn't expose them.

## Data plane v1

v1 implements the same contract trivially. It shares RDS, so `ApplyConfig` is a no-op ack and the feed is empty. Its gateway is today's server, and the edge sends v1 orgs' session traffic there. Moving an org from v1 to v2 is: enable the org, send a full snapshot, import its sessions, then the edge starts routing the org to v2.
