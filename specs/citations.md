# Citations Specification

## Abstract

Citations attach **claim-level provenance** to assistant messages: a span of
generated text is linked to the source that backs it, so a reader (or an eval)
can trace a claim to its evidence and verify it. Citations are delivered as
**capabilities**, not as a single built-in feature. Multiple, independently
enable/disable-able citation capabilities can coexist on one agent — one for
retrieval-backed answers, one for provider-native document citations, one for
web results — and a separate guardrail capability verifies them. Because every
citation capability writes into the **same thin render contract** (an annotation
on message text), the UI renders them uniformly and evals can hold the feed
fixed while varying the verifier, or compare two feeds head-to-head.

This spec is the durable design intent. Implementation lands across the phases
in [Phasing](#phasing).

## Motivation

Grounding already exists in everruns, but it dies as opaque tool-result JSON:
`search_index` returns `KnowledgeIndexCitation` (`crates/core/src/vector_store.rs`)
and `search_knowledge` returns `KnowledgeSearchHit` (`crates/core/src/traits.rs`),
each carrying a stable id (`kchk_…` / `kbe_…`), `source_uri`, `location`, and a
`snippet`. But nothing ties those sources to the *specific sentence* the model
wrote, nothing renders them as a linkable affordance (there is no citation UI
component today), and provider-native citations (Anthropic's Citations API —
the `citations` capability flag is detected at `crates/anthropic/src/driver.rs`
but unused) are not wired in. The public OpenResponses API already models inline
citations (`OutputText.annotations: Vec<UrlCitation>` in
`crates/core/src/openresponses_types.rs`) but the internal chat path never
populates it, and it is URL-only.

The goal is to close that gap **without** forcing every producer into one domain
model. A RAG chunk, a provider document span, a web result, and a future
SQL-row provenance have different natural fields; making them agree on a single
`Citation` struct freezes the design and blocks new sources. Instead, unify only
the rendering contract and let each producer own its own representation.

## Non-goals

* A unified `Citation` domain type. `KnowledgeIndexCitation` and
  `KnowledgeSearchHit` stay as they are; each capability maps its own type into
  the shared render envelope at emit time.
* Backfilling citations onto historical messages. Annotations attach to newly
  generated messages only.
* Image citations. Text spans only (matching Anthropic's current limitation).

## Design

### The narrow waist: one render contract, not one domain model

The only shared type is a **rendering envelope** attached to generated text.
Producers agree on "here is a text span and a thing to link to" — nothing about
citation *semantics*.

* A new optional field `annotations` on `TextContentPart`
  (`crates/core/src/message.rs`), serialized `skip_serializing_if` empty so the
  wire shape of non-cited text is unchanged. It derives the `openapi` `ToSchema`
  so the TypeScript types regenerate automatically (no hand-written TS).
* A new `TextAnnotation` struct. It is deliberately minimal and open:
  * a character `span` (start/end, exclusive end) into the enclosing
    `TextContentPart.text`;
  * an `origin` = the producing capability id, so the UI and evals can attribute
    and filter each chip by feed;
  * a `source` reference: `uri`, optional `title`, optional `snippet`, optional
    `location` (reusing the existing `location` JSONB shape — line/char/page/
    block ranges);
  * an opaque `external_id` (`kchk_…` / `kbe_…` / url-hash / provider index) the
    waist never interprets;
  * an optional `verified` verdict, filled in later by the verification
    capability (absent = unverified).

This mirrors, and generalizes beyond URLs, the existing
`OutputText.annotations` model in `openresponses_types.rs`; the OpenResponses API
surface maps onto it.

### Citation approaches are capabilities

Each approach is a `Capability` (`crates/core/src/capabilities/`), registered in
`CapabilityRegistry::with_builtins_for_grade`, using seams that already exist on
the trait. They coexist because they all emit into the annotation envelope,
tagged by `origin`.

| Capability | Feed | Emission mechanism |
| --- | --- | --- |
| `citation_retrieval` | knowledge-index / knowledge-base tool results | reads citations off `search_index` / `search_knowledge` results (`post_tool_exec_hooks`), then attaches spans via the post-generation annotation hook (below) by matching claims to retrieved snippets |
| `citation_native` | Anthropic Citations API (`search_result` / `document` blocks) | the Anthropic driver emits `citations.enabled` source blocks and parses `citations_delta` / text-block `citations[]` back into annotations directly — token-free, pointer-guaranteed |
| `citation_web` | provider server-side web search (`openrouter_server_tools`) | maps URL results into `source.uri` |

Per-capability config lives in `config_schema()` / `validate_config()`; the UI
renders it generically. `features()` returns `"citations"` so the citation UI
surface only appears when some citation capability is active.

### The post-generation annotation seam

The reason atom already runs end-of-message guardrails on the fully-assembled
assistant text before the `Message` is built
(`evaluate_post_generation_guardrails` in `crates/core/src/atoms/reason.rs`,
trait `PostGenerationOutputGuardrail` in `crates/core/src/output_guardrail.rs`).
Those are **block/allow only**. Citations need a **mutating sibling in the same
family**: a `PostGenerationAnnotationHook` that receives the assembled text (and
an LLM-capable context) and returns `Vec<TextAnnotation>` to attach to the
`TextContentPart` before the `output.message.completed` event is emitted. It
runs alongside the existing guardrail seam and is assembled per-capability via a
new trait method (mirroring `post_output_guardrails_with_config`). This is the
one net-new platform seam; everything else reuses existing hooks.

`citation_native` does not use this seam — its annotations arrive inline from the
provider stream and are attached during driver parsing (a new `LlmStreamEvent`
variant carries citation deltas up to the reason atom).

### Verification is a separate, composable guardrail capability

`citation_verification` is a standalone capability with `is_guardrail() = true`.
It consumes annotations produced by **any** citation feed, runs an NLI-style
entailment check (claim = hypothesis, `source.snippet` = premise) using a
configured utility model, and stamps each annotation's `verified` verdict
(`entailed` / `unsupported` / `uncertain`, with a score). It is **off by
default** (it costs an extra model call) and configured via `config_schema`
(`verifier` binding, `threshold`). Keeping it decoupled from the feeds means any
feed can be paired with any verifier, and evals can vary one axis at a time.

Citation capabilities expose a lightweight `verify: bool` in their own config as
an ergonomic switch that simply implies the `citation_verification` dependency;
the verification logic itself is never duplicated per feed.

### UI

Gated by the `"citations"` feature. A component under
`apps/ui/src/components/chat/` consumes the annotation envelope regardless of
`origin`: inline numbered chips at each annotated span, a hover popover
(`title` / `snippet` / `uri`), a deduped source strip below the message, click
-through to `uri`, and a "verified" badge when a `verified` verdict is present.
Assistant text currently reaches the UI as a plain string
(`streamdown-message.tsx`); rendering chips requires threading the structured
`annotations` down (pre-splitting spans, mirroring the existing
`splitA2UIBlocks` approach) rather than relying on markdown text. Streamdown
continues to render the surrounding markdown.

### Persistence and export

Annotations ride the append-only event model: they are part of the
`Message` inside `output.message.completed` (`crates/core/src/events.rs`), so
they survive reload, forking, and session export
(`specs/session-export.md`) with no separate store. `cited_text` from native
providers is not re-sent on subsequent turns (Anthropic does not bill it), so we
keep `snippet` for display but never rely on it for prompt reconstruction.

### Evals and comparison

Because each feed is a capability with the same output contract, a citation
eval is two agents identical except for the enabled `citation_*` capability,
scored on citation faithfulness and coverage. A scorer
(`crates/server/src/domains/evals/`, `specs/evals.md`) grades annotated messages
using the `verified` verdicts plus a held-out NLI judge. This makes citation
approaches directly benchmarkable — the payoff of keeping the waist thin.

## Security and privacy

* `snippet` / `cited_text` can leak source content into transcripts and exports;
  citations are subject to the same tool-output distillation / guardrail paths
  as their underlying retrieval (`specs/tool-output-distillation.md`).
* Retrieval feeds must preserve org-scoping: an annotation's `source` must never
  reference a document the requesting org cannot read. Feeds derive `source`
  only from already-authorized retrieval results (org-scoped by construction in
  `KnowledgeIndexSearch`).
* The verifier is a utility-model call over already-retrieved text; it adds no
  new data egress beyond what the feed already surfaced.

## Phasing

1. **Waist** — `TextAnnotation` + optional `annotations` on `TextContentPart`;
   the `PostGenerationAnnotationHook` seam in the reason atom; regenerated TS
   types. No behavior change until a capability emits.
2. **`citation_retrieval`** capability over existing `search_index` /
   `search_knowledge`, plus the UI component and `features()` gate. First
   end-to-end path, no provider work.
3. **`citation_native`** — Anthropic `search_result`/`document` blocks +
   `citations_delta` parsing + `LlmStreamEvent` carriage. Proves the
   multi-capability, same-contract design with a distinct feed.
4. **`citation_verification`** guardrail capability + the citation eval scorer.
5. **`citation_web`** and additional feeds as sources land.

## Open questions

* Chip numbering across multiple concurrent feeds — global sequential vs.
  per-origin namespaced. Leaning global-sequential, deduped by `uri`.
* Whether `citation_retrieval` should attach spans by deterministic string match
  only, or fall back to an LLM span-alignment pass when the model paraphrases
  retrieved text (cost vs. coverage).
* Whether to expose annotations on the public OpenResponses API immediately or
  only after the native feed lands (the `annotations` slot already exists there).
