import { z } from "zod";
import type { ObserverScorerScope } from "@/lib/api/types";
import { draftText } from "./use-page-form-draft";

const weight = z.number().nonnegative().optional();
const count = z.number().int().nonnegative();
const rule = z.discriminatedUnion("type", [
  z.strictObject({ type: z.literal("contains"), text: draftText, weight }),
  z.strictObject({ type: z.literal("not_contains"), text: draftText, weight }),
  z.strictObject({ type: z.literal("regex"), pattern: draftText, weight }),
  z.strictObject({
    type: z.literal("tool_called"),
    tool: draftText,
    min: count.optional(),
    weight,
  }),
  z.strictObject({ type: z.literal("tool_not_called"), tool: draftText, weight }),
  z.strictObject({
    type: z.literal("tool_call_count"),
    min: count.optional(),
    max: count.optional(),
    weight,
  }),
  z.strictObject({ type: z.literal("turns_within"), max: count, weight }),
  z.strictObject({
    type: z.literal("json_schema"),
    schema: z.record(z.string(), z.json()),
    weight,
  }),
]);
const scope: z.ZodType<ObserverScorerScope> = z.literal("turn");
const common = { key: draftText, scope };
export const observerDraftSchema = z.strictObject({
  name: draftText,
  description: draftText,
  sampling_rate: z.number().min(0).max(1),
  agent_ids: z.array(draftText).max(100),
  harness_ids: z.array(draftText).max(100),
  session_tags: draftText,
  scorers: z
    .array(
      z.discriminatedUnion("method", [
        z.strictObject({ ...common, method: z.literal("rule"), rule }),
        z.strictObject({
          ...common,
          method: z.literal("llm_judge"),
          rubric: draftText,
          model_id: draftText.optional(),
          pass_threshold: z.number().min(0).max(1),
        }),
      ]),
    )
    .max(100),
});

// Preserve API definitions beyond the typed editor schema without rewriting
// them while editing metadata. Their scorer collection stays read-only.
const readRule = z.discriminatedUnion("type", [
  z.strictObject({
    type: z.literal("json_schema"),
    schema: z.union([z.record(z.string(), z.json()), z.boolean()]),
    weight,
  }),
  ...rule.options.filter((option) => option.shape.type.value !== "json_schema"),
  z.strictObject({
    type: z.literal("citation_faithful"),
    min_citations: count,
    pass_threshold: z.number().min(0).max(1),
    weight,
  }),
  z.strictObject({
    type: z.literal("citation_judged"),
    rubric: draftText.optional(),
    model_id: draftText.optional(),
    pass_threshold: z.number().min(0).max(1),
    weight,
  }),
]);
export const observerReadScorersSchema = observerDraftSchema.extend({
  scorers: z
    .array(
      z.discriminatedUnion("method", [
        z.strictObject({ ...common, method: z.literal("rule"), rule: readRule }),
        observerDraftSchema.shape.scorers.element.options[1],
      ]),
    )
    .max(100),
});
const writableRuleTypes = new Set<string>(rule.options.map((option) => option.shape.type.value));
export function canEditObserverScorers(scorers: readonly unknown[]): boolean {
  return scorers.every((value) => {
    if (!value || typeof value !== "object") return false;
    const scorer = value as {
      scope?: unknown;
      method?: unknown;
      rule?: { type?: unknown; schema?: unknown };
    };
    if (scorer.scope !== "turn") return false;
    if (scorer.method === "llm_judge") return true;
    const candidate = scorer.rule;
    return (
      scorer.method === "rule" &&
      typeof candidate?.type === "string" &&
      writableRuleTypes.has(candidate.type) &&
      (candidate.type !== "json_schema" ||
        (!!candidate.schema &&
          typeof candidate.schema === "object" &&
          !Array.isArray(candidate.schema)))
    );
  });
}
