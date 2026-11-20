/** Product labels shared by chat, inspection and capability settings. */
export const COMPACTION_STRATEGIES: Record<string, string> = {
  auto: "Automatic",
  native: "Provider-native",
  provider_managed: "Provider-native",
  observation_masking: "Observation masking",
  masking: "Observation masking",
  summarization: "Portable summary",
  portable_summary: "Portable summary",
  aggressive_trim: "History trimming",
  trimming: "History trimming",
};

export function compactionStrategyLabels(strategy: string): string[] {
  return strategy.split("+").map((part) => COMPACTION_STRATEGIES[part] ?? "Context reduction");
}
