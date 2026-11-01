// The Evals section tabs, addressed by `?tab=`.

export const EVALS_TABS = [
  "overview",
  "datasets",
  "experiments",
  "observers",
  "findings",
  "graders",
] as const;
export type EvalsTab = (typeof EVALS_TABS)[number];

export function resolveEvalsTab(value: string | null): EvalsTab {
  return (EVALS_TABS as readonly string[]).includes(value ?? "") ? (value as EvalsTab) : "overview";
}
