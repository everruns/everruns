// Shared formatting helpers for eval result views (run-detail table + matrix),
// so they agree on target labels and score normalization.

import type { EvalCaseResult, EvalScore, EvalTarget } from "@/lib/api/types";

/// Human label for a target across all variants. External (imported) targets
/// read as `provider/model`; session/app keep their existing labels.
export function targetLabel(target?: EvalTarget): string {
  if (!target) return "—";
  if (target.type === "app") return `app: ${target.app_id}`;
  if (target.type === "external") {
    return target.model ? `${target.provider}/${target.model}` : target.provider;
  }
  return "session";
}

export interface NormalizedScore {
  name: string;
  pass: boolean;
  value: number;
  reason?: string;
}

/// Scores persist as a named array (`[{scorer, value, pass, reason}]`) for both
/// external imports and write-back; older rows may use a keyed object. Normalize
/// to one shape so views render scorer names either way.
export function normalizeScores(scores: EvalCaseResult["scores"]): NormalizedScore[] {
  if (!scores) return [];
  if (Array.isArray(scores)) {
    return scores.map((s: EvalScore, i) => ({
      name: s.scorer ?? `scorer ${i + 1}`,
      pass: s.pass,
      value: s.value,
      reason: s.reason,
    }));
  }
  return Object.entries(scores).map(([name, s]) => ({
    name,
    pass: s.pass,
    value: s.value,
    reason: s.reason,
  }));
}

/// Mean of applicable score values for a result, or null when none are numeric.
export function meanScore(scores: EvalCaseResult["scores"]): number | null {
  const values = normalizeScores(scores)
    .map((s) => s.value)
    .filter((v) => typeof v === "number" && Number.isFinite(v));
  if (values.length === 0) return null;
  return values.reduce((a, v) => a + v, 0) / values.length;
}
