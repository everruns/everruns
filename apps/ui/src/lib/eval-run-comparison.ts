export const MAX_COMPARE_RUNS = 8;

const EVAL_RUN_ID_PATTERN = /^evalrun_[0-9a-f]{32}$/;

export type ParsedCompareRunIds = {
  runIds: string[];
  ignoredCount: number;
  exceededLimit: boolean;
};

export function parseCompareRunIds(value: string | null): ParsedCompareRunIds {
  const runIds: string[] = [];
  const seen = new Set<string>();
  let ignoredCount = 0;
  let exceededLimit = false;

  for (const rawRunId of (value ?? "").split(",")) {
    const runId = rawRunId.trim();
    if (!runId) {
      continue;
    }

    if (!EVAL_RUN_ID_PATTERN.test(runId) || seen.has(runId)) {
      ignoredCount += 1;
      continue;
    }

    if (runIds.length >= MAX_COMPARE_RUNS) {
      ignoredCount += 1;
      exceededLimit = true;
      continue;
    }

    seen.add(runId);
    runIds.push(runId);
  }

  return { runIds, ignoredCount, exceededLimit };
}
