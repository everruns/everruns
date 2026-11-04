// Hosts supply their billing rates. Unknown prices remain unknown; no estimator
// input writes to a ledger or changes what a provider charges.
export type CostBilling = "credits" | "external";
export interface OperatingCostLine {
  id: string;
  label: string;
  unit: string;
  quantity: number;
  usdPerUnit?: number;
  billing: CostBilling;
  period: "task" | "month";
  rateId?: string;
}
export interface CostRate {
  id: string;
  label: string;
  unit: string;
  usdPerUnit: number;
  billing: CostBilling;
  period?: "task" | "month";
}
export interface AgentCostPricing {
  status: "ready" | "loading" | "error";
  modelBilling: CostBilling;
  creditLabel?: string;
  rates: CostRate[];
  asOf?: string;
}
type Bucket = {
  perTask: number | null;
  fixedMonthly: number | null;
  daily: number | null;
  monthly: number | null;
};

export function projectOperatingCosts(
  modelPerTask: number | null,
  modelBilling: CostBilling,
  lines: OperatingCostLine[],
  dailyTasks: number,
) {
  const valid = (n: number) => Number.isFinite(n) && n >= 0;
  if (
    !valid(dailyTasks) ||
    dailyTasks > 1_000_000_000 ||
    (modelPerTask !== null && !valid(modelPerTask)) ||
    lines.length > 20 ||
    lines.some(
      (line) =>
        !valid(line.quantity) ||
        line.quantity > 1_000_000_000 ||
        (line.usdPerUnit !== undefined && (!valid(line.usdPerUnit) || line.usdPerUnit > 1_000_000)),
    )
  )
    return null;
  const unknown: string[] = [];
  const empty = (): Bucket => ({ perTask: 0, fixedMonthly: 0, daily: 0, monthly: 0 });
  const buckets = { credits: empty(), external: empty() };
  buckets[modelBilling].perTask = modelPerTask;
  if (modelPerTask === null && dailyTasks > 0) unknown.push("Model usage");
  for (const line of lines) {
    if (line.quantity === 0) continue;
    const bucket = buckets[line.billing];
    const key = line.period === "task" ? "perTask" : "fixedMonthly";
    if (line.usdPerUnit === undefined) {
      bucket[key] = null;
      if (line.period === "month" || dailyTasks > 0) unknown.push(line.label);
    } else if (bucket[key] !== null) bucket[key] += line.quantity * line.usdPerUnit;
  }
  const add = (a: number | null, b: number | null) => (a === null || b === null ? null : a + b);
  for (const bucket of Object.values(buckets)) {
    const variable =
      dailyTasks === 0 ? 0 : bucket.perTask === null ? null : bucket.perTask * dailyTasks * 30;
    bucket.monthly = add(variable, bucket.fixedMonthly);
    bucket.daily = bucket.monthly === null ? null : bucket.monthly / 30;
  }
  const total: Bucket = {
    perTask: add(buckets.credits.perTask, buckets.external.perTask),
    fixedMonthly: add(buckets.credits.fixedMonthly, buckets.external.fixedMonthly),
    daily: add(buckets.credits.daily, buckets.external.daily),
    monthly: add(buckets.credits.monthly, buckets.external.monthly),
  };
  if (
    [...Object.values(buckets), total].some((bucket) =>
      Object.values(bucket).some((value) => value !== null && !valid(value)),
    )
  )
    return null;
  return { ...buckets, total, complete: unknown.length === 0, unknown };
}
