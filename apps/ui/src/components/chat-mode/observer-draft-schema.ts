import { z } from "zod";
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
const common = { key: draftText, scope: z.enum(["turn", "session", "tool"]) };
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
