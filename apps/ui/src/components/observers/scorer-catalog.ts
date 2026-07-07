// Starter llm_judge scorer templates the user can add with one click.
// Each template maps to a `method: "llm_judge"` scorer config when added.

export interface ScorerTemplate {
  /** Unique scorer key, used as the catalog identifier and the scorer key. */
  key: string;
  /** Short human label shown on the add button. */
  label: string;
  /** One-line description of what the scorer measures. */
  description: string;
  /** Judge rubric prompt. */
  rubric: string;
  /** Pass threshold (0–1). */
  pass_threshold: number;
}

export const SCORER_CATALOG: ScorerTemplate[] = [
  {
    key: "answer_completeness",
    label: "Answer completeness",
    description: "How fully the answer addresses the user's question.",
    rubric:
      "Score 1.0 if the answer fully and directly addresses the user's question; lower if partial or off-topic.",
    pass_threshold: 0.5,
  },
  {
    key: "source_citation",
    label: "Source citation",
    description: "Whether claims are backed by cited sources or tool results.",
    rubric: "Score 1.0 if claims are backed by cited sources/tool results; 0.0 if unsupported.",
    pass_threshold: 0.5,
  },
  {
    key: "user_frustration",
    label: "User frustration",
    description: "Whether the user seems satisfied or frustrated.",
    rubric:
      "Score 1.0 if the user seems satisfied; lower if they show frustration or repeat themselves.",
    pass_threshold: 0.5,
  },
  {
    key: "hallucination_risk",
    label: "Hallucination risk",
    description: "Whether the answer invents facts not supported by context.",
    rubric:
      "Score 1.0 if the answer stays grounded in the provided context and tool results; lower the more it asserts facts that are not supported, fabricated, or contradicted by available evidence.",
    pass_threshold: 0.5,
  },
  {
    key: "tool_error_rate",
    label: "Tool error rate",
    description: "Whether tools were used successfully without errors.",
    rubric:
      "Score 1.0 if any tools used in the turn completed successfully and their results were handled correctly; lower if tools errored, were retried excessively, or their failures were ignored.",
    pass_threshold: 0.5,
  },
];
