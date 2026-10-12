import type { TriggerEventFilter } from "@/lib/api/schema-types";

type Condition = NonNullable<TriggerEventFilter["conditions"]>[number];

// CI filters on a GitHub trigger. The server normalizes workflow_run and
// check_suite events into `github.ci.*`, so a trigger narrows CI by matching
// those paths: one condition per field, any listed value passes.

/** Where CI events carry their normalized result. */
export const CI_OUTCOME_PATH = "github.ci.outcome";
export const CI_BRANCH_PATH = "github.ci.branch";
export const CI_WORKFLOW_PATH = "github.ci.workflow";

/** String values a filter accepts at `path`; empty when the path is not filtered. */
export function conditionValues(filter: TriggerEventFilter | null | undefined, path: string) {
  const condition = filter?.conditions?.find((candidate) => candidate.path === path);
  // The generated type says objects; the server accepts any JSON value.
  const values = (condition?.any_of ?? []) as unknown[];
  return values.filter((value): value is string => typeof value === "string");
}

/** The filter with the condition at `path` replaced; no values removes it. */
export function withCondition(
  filter: TriggerEventFilter | null | undefined,
  path: string,
  values: string[],
): TriggerEventFilter {
  const kept = (filter?.conditions ?? []).filter((condition) => condition.path !== path);
  const cleaned = [...new Set(values.map((value) => value.trim()).filter(Boolean))];
  return {
    ...filter,
    conditions:
      cleaned.length > 0
        ? [...kept, { path, any_of: cleaned as unknown as Condition["any_of"] }]
        : kept,
  };
}

/** A CI filter from the builder's draft, or undefined when it filters nothing. */
export function ciFilter(options: {
  changesOnly: boolean;
  branches?: string[];
  workflows?: string[];
}): TriggerEventFilter | undefined {
  let filter = withCondition(null, CI_BRANCH_PATH, options.branches ?? []);
  filter = withCondition(filter, CI_WORKFLOW_PATH, options.workflows ?? []);
  if (options.changesOnly) filter = { ...filter, changes_only: { path: CI_OUTCOME_PATH } };
  return (filter.conditions?.length ?? 0) > 0 || filter.changes_only ? filter : undefined;
}

/** Comma-separated text as a list, for the branch and workflow fields. */
export function splitList(text: string): string[] {
  return text
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}
