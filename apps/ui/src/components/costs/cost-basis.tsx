"use client";

// Every amount shown on the Costs pages says where its number came from.
// Unpriced usage is shown as unknown, never as $0.

import { Badge } from "@/components/ui/badge";
import type { CostBasis } from "@/lib/api/costs";
import { HOST_BILLED_DESCRIPTION, HOST_BILLED_LABEL } from "@/lib/cost-host-label";
import { cn } from "@/lib/utils";
import { useUiTranslations } from "@/providers/locale-provider";

export const BASIS_LABELS: Record<CostBasis, string> = {
  host: HOST_BILLED_LABEL,
  rate: "Your rate",
  provider: "Provider reported",
  list_price: "List price estimate",
  unpriced: "Not priced",
  mixed: "Mixed",
};

export const BASIS_DESCRIPTIONS: Record<Exclude<CostBasis, "mixed">, string> = {
  host: HOST_BILLED_DESCRIPTION,
  rate: "A rate from your price list.",
  provider: "The provider sent its cost with the response (OpenRouter does).",
  list_price: "The model catalog price for the model, cache-aware.",
  unpriced: "No price anywhere. Counted as usage, cost unknown, never shown as $0.",
};

const BASIS_CLASSES: Record<CostBasis, string> = {
  host: "border-warning/40 bg-warning/10 text-foreground",
  rate: "border-info/40 bg-info/10 text-foreground",
  provider: "border-success/30 bg-success/12 text-success",
  list_price: "border-border bg-muted text-muted-foreground",
  unpriced: "border-destructive/40 bg-destructive/10 text-destructive",
  mixed: "border-border text-muted-foreground",
};

export function BasisBadge({ basis, className }: { basis: CostBasis; className?: string }) {
  const ui = useUiTranslations();
  return (
    <Badge variant="outline" className={cn(BASIS_CLASSES[basis], className)}>
      {ui(BASIS_LABELS[basis])}
    </Badge>
  );
}

export function formatUsd(amount: number): string {
  if (amount !== 0 && Math.abs(amount) < 0.01) return `$${amount.toFixed(4)}`;
  return `$${amount.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
}

export function formatQuantity(quantity: number, unit: "tokens" | "hours"): string {
  if (unit === "hours") return `${quantity.toFixed(quantity < 10 ? 1 : 0)} h`;
  if (quantity >= 1_000_000) return `${(quantity / 1_000_000).toFixed(1)}M tokens`;
  if (quantity >= 1_000) return `${(quantity / 1_000).toFixed(1)}K tokens`;
  return `${Math.round(quantity)} tokens`;
}
