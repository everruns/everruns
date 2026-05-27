# LLM-specific OpenAPI extensions (`x-llm-*`)

Closed vocabulary of custom OpenAPI extension fields that encode
LLM-relevant operation metadata not expressible in standard OpenAPI.
Lets an agent toolcaller reason about **cost, safety, and preference**
before picking between similar endpoints — without out-of-band docs.

The vocabulary is intentionally small (six fields), additive, and
**namespace-clean**: every key starts with `x-` so standard OpenAPI
consumers ignore them unchanged.

## Vocabulary

| Field             | Type             | Closed values                            | Default if unset       | Purpose                                                                            |
| ----------------- | ---------------- | ---------------------------------------- | ---------------------- | ---------------------------------------------------------------------------------- |
| `x-side-effect`   | enum             | `none` / `reversible` / `irreversible`   | `reversible`           | Worst-case mutation surface of the operation.                                       |
| `x-cost-tier`     | enum             | `free` / `paid`                          | `free`                 | Whether invoking the operation can incur LLM token spend or other billed services. |
| `x-llm-prefer`    | enum             | `prefer` / `neutral` / `avoid`           | `neutral`              | Routing hint when multiple operations achieve a similar outcome.                    |
| `x-llm-rationale` | string           | free-form, one sentence                  | (omitted)              | Surfaces alongside `x-llm-prefer != neutral` to explain the recommendation.        |
| `x-error-codes`   | array of strings | `ErrorResponse.code` values              | (omitted)              | Closed set of `code` values this operation can emit.                                |
| `x-superseded-by` | string           | OpenAPI `operationId`                    | (omitted)              | Recommended successor when this operation is deprecated.                            |

Untagged operations inherit defaults — an agent only needs to read
fields that are present.

## When to add each tag

* **`x-side-effect`** — set on every mutating operation (`POST`,
  `PUT`, `PATCH`, `DELETE`). Use `irreversible` on hard-delete /
  rotate-credentials / destructive bash; `reversible` on soft-delete,
  archive, pause, unarchive, undo-eligible writes.
* **`x-cost-tier`** — set `paid` on any operation that drives LLM
  inference (`agent_run`, `session_send_message`, voice sessions,
  `discover`/`execute` when they invoke an agent). Default `free` is
  fine for everything else.
* **`x-llm-prefer`** — set `avoid` on hard-delete or other
  semantically-destructive operations that have a safer alternative
  (e.g. `delete_agent` → recommend `archive` instead). Set `prefer`
  on the cheap/safe sibling when two operations overlap. Pair with
  `x-llm-rationale` whenever non-`neutral`.
* **`x-llm-rationale`** — one sentence, present tense, addresses the
  caller. "Use POST /v1/agents/{id}/archive instead unless audit
  trail must be removed."
* **`x-error-codes`** — set when the operation has a closed set of
  `ErrorResponse.code` values it can return. Skip on operations
  that re-emit upstream errors verbatim (no closed taxonomy).
* **`x-superseded-by`** — set when the operation is deprecated and
  there's a drop-in replacement. Value is the successor's
  `operationId`. Pair with the standard OpenAPI `deprecated: true`.

## Where defaults stop and tags start

Tagging is not exhaustive — only operations that **differ from the
default** need a tag. An LLM client treats absent tags as the
default value, so leaving "reversible / free / neutral" implicit is
the convention.

## utoipa wiring

`utoipa::path` accepts an `extensions(...)` attribute (utoipa ≥ 5.4).
Use it directly on the handler:

```rust
#[utoipa::path(
    delete,
    path = "/v1/agents/{id}",
    extensions(
        ("x-side-effect" = json!("irreversible")),
        ("x-llm-prefer" = json!("avoid")),
        ("x-llm-rationale" = json!(
            "Use archive_agent instead unless the agent must be removed from \
             the audit trail."
        )),
    ),
    // …
)]
pub async fn delete_agent(…) { … }
```

Extensions are emitted directly under the operation object in
`docs/api/openapi.json` and survive `scripts/export-openapi.sh`
unchanged.

### Deterministic ordering caveat

utoipa 5.4 stores `extensions(...)` entries in a `HashMap` internally,
so multiple entries on a single operation serialize in a
non-deterministic order. The `openapi-check` CI job diffs the
committed spec against a fresh re-export, so non-deterministic
ordering breaks the freshness gate.

Workaround until utoipa stabilises: **emit only one extension per
operation**. Encode anything you'd want to convey as a second entry
either as the value of a more general tag, or simply omit it when
the value would match the default (e.g. `x-side-effect: reversible`
is the default for non-DELETE operations and can be left off
implicitly). Multi-entry extensions on the same operation are
tracked as a follow-on once utoipa switches to a deterministic
container.

## First-wave rollout

This convention lands with a small first wave so the vocabulary can
stabilise before broad application. A separate follow-on issue
tracks the per-endpoint sweep + a ratchet test
(`MIN_OP_LLM_HINTS_PCT`) that ensures coverage doesn't regress.
