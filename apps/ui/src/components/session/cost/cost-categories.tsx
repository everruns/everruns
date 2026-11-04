"use client";

// "Where the money went": total spend and its split by cost category.

import type { SessionCost } from "@/lib/api/types";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";
import {
  CATEGORY_BG,
  CATEGORY_ORDER,
  type CostCategoryKey,
  CostCard,
  ShareBar,
  Swatch,
  formatCompact,
  formatShare,
  formatUsd,
} from "./cost-shared";

/** What cached reads would have cost at the uncached rate, minus what they cost. */
export function cacheSavings(cost: SessionCost): number {
  const uncached = cost.categories.find((c) => c.key === "uncached_input");
  const read = cost.categories.find((c) => c.key === "cache_read");
  if (!uncached || !read || uncached.tokens === 0 || read.tokens === 0) return 0;
  const uncachedRate = uncached.usd / uncached.tokens;
  return Math.max(0, read.tokens * uncachedRate - read.usd);
}

export function promptTokens(cost: SessionCost): number {
  return cost.categories.filter((c) => c.key !== "output").reduce((sum, c) => sum + c.tokens, 0);
}

export function CostCategories({ cost, modelLabel }: { cost: SessionCost; modelLabel?: string }) {
  const { displayLocale: locale } = useLocale();
  const ui = useUiTranslations();
  const total = cost.total_usd;
  const prompt = promptTokens(cost);

  const labels: Record<CostCategoryKey, string> = {
    uncached_input: ui("Prompt, uncached"),
    cache_read: ui("Prompt, read from cache"),
    cache_write: ui("Prompt, written to cache"),
    output: ui("Output"),
  };
  const notes: Record<CostCategoryKey, string> = {
    uncached_input: ui("Billed at the full input rate"),
    cache_read:
      prompt > 0
        ? ui("{value0} of the prompt", {
            value0: formatShare(categoryTokens(cost, "cache_read"), prompt, locale),
          })
        : "",
    cache_write: ui("Stored for later calls to read"),
    output: ui("Includes reasoning"),
  };
  const savings = cacheSavings(cost);

  const meta = [
    modelLabel,
    ui("{value0} model calls", { value0: cost.model_calls.toLocaleString(locale) }),
  ]
    .filter(Boolean)
    .join(" · ");

  return (
    <CostCard title={ui("Where the money went")} meta={meta}>
      <div className="flex flex-wrap items-end gap-6 px-4 pt-4 pb-3">
        <div>
          <div className="text-xs text-muted-foreground">{ui("Total spend")}</div>
          <div className="text-3xl font-semibold tracking-tight tabular-nums">
            {formatUsd(total, locale)}
          </div>
        </div>
        <div className="flex h-3.5 min-w-48 flex-1 gap-0.5 self-center" aria-hidden>
          {CATEGORY_ORDER.map((key) => {
            const usd = categoryUsd(cost, key);
            return usd > 0 ? (
              <span key={key} className={CATEGORY_BG[key]} style={{ flexGrow: usd, minWidth: 3 }} />
            ) : null;
          })}
          {cost.unattributed_usd > 0 ? (
            <span
              className={CATEGORY_BG.unattributed}
              style={{ flexGrow: cost.unattributed_usd, minWidth: 3 }}
            />
          ) : null}
        </div>
      </div>
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-border text-xs text-muted-foreground">
              <th className="px-4 py-2 text-left font-medium">{ui("Category")}</th>
              <th className="px-4 py-2 text-right font-medium">{ui("Tokens")}</th>
              <th className="px-4 py-2 text-right font-medium">{ui("Average rate")}</th>
              <th className="px-4 py-2 text-right font-medium">{ui("Spend")}</th>
              <th className="px-4 py-2 text-right font-medium">{ui("Share")}</th>
            </tr>
          </thead>
          <tbody>
            {CATEGORY_ORDER.map((key) => {
              const category = cost.categories.find((c) => c.key === key);
              const tokens = category?.tokens ?? 0;
              const usd = category?.usd ?? 0;
              const idle = tokens === 0;
              const note =
                key === "cache_read" && savings >= 0.01
                  ? `${notes[key]} · ${ui("saved {value0}", { value0: formatUsd(savings, locale) })}`
                  : notes[key];
              return (
                <tr
                  key={key}
                  className={`border-b border-border/60 last:border-b-0 ${idle ? "text-muted-foreground" : ""}`}
                >
                  <td className="px-4 py-2.5">
                    <div className="flex items-center gap-2">
                      <Swatch className={CATEGORY_BG[key]} />
                      <span className="font-medium">{labels[key]}</span>
                    </div>
                    {note && !idle ? (
                      <div className="ml-[18px] text-xs text-muted-foreground">{note}</div>
                    ) : null}
                  </td>
                  <td className="px-4 py-2.5 text-right tabular-nums">
                    {idle ? "—" : formatCompact(tokens, locale)}
                  </td>
                  <td className="px-4 py-2.5 text-right text-xs text-muted-foreground tabular-nums">
                    {idle || usd === 0
                      ? "—"
                      : ui("{value0} / M", {
                          value0: formatUsd((usd / tokens) * 1_000_000, locale),
                        })}
                  </td>
                  <td className="px-4 py-2.5 text-right font-medium tabular-nums">
                    {idle ? "—" : formatUsd(usd, locale)}
                  </td>
                  <td className="px-4 py-2.5 text-right tabular-nums whitespace-nowrap">
                    <ShareBar share={total > 0 ? usd / total : 0} className={CATEGORY_BG[key]} />
                    <span className="ml-2 inline-block w-12">
                      {idle ? "—" : formatShare(usd, total, locale)}
                    </span>
                  </td>
                </tr>
              );
            })}
            {cost.unattributed_usd > 0 ? (
              <tr>
                <td className="px-4 py-2.5">
                  <div className="flex items-center gap-2">
                    <Swatch className={CATEGORY_BG.unattributed} />
                    <span className="font-medium">{ui("Not split by category")}</span>
                  </div>
                  <div className="ml-[18px] text-xs text-muted-foreground">
                    {ui("The provider reported a cost for a model without a price table")}
                  </div>
                </td>
                <td className="px-4 py-2.5 text-right">—</td>
                <td className="px-4 py-2.5 text-right">—</td>
                <td className="px-4 py-2.5 text-right font-medium tabular-nums">
                  {formatUsd(cost.unattributed_usd, locale)}
                </td>
                <td className="px-4 py-2.5 text-right tabular-nums whitespace-nowrap">
                  <ShareBar
                    share={cost.unattributed_usd / total}
                    className={CATEGORY_BG.unattributed}
                  />
                  <span className="ml-2 inline-block w-12">
                    {formatShare(cost.unattributed_usd, total, locale)}
                  </span>
                </td>
              </tr>
            ) : null}
          </tbody>
        </table>
      </div>
      {cost.unpriced_calls > 0 ? (
        <p className="border-t border-border px-4 py-2.5 text-xs text-muted-foreground">
          {ui(
            "{value0} model calls have no price, so their tokens are counted but not their cost.",
            {
              value0: cost.unpriced_calls.toLocaleString(locale),
            },
          )}
        </p>
      ) : null}
    </CostCard>
  );
}

function categoryUsd(cost: SessionCost, key: CostCategoryKey): number {
  return cost.categories.find((c) => c.key === key)?.usd ?? 0;
}

function categoryTokens(cost: SessionCost, key: CostCategoryKey): number {
  return cost.categories.find((c) => c.key === key)?.tokens ?? 0;
}
