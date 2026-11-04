"use client";

// This month's cost of one agent, from the org's cost record, with a way to
// the full picture. Unpriced usage makes the total partial, never zero.

import Link from "next/link";
import { useQuery } from "@tanstack/react-query";

import { formatUsd } from "@/components/costs/cost-basis";
import { getAgentCost } from "@/lib/api/costs";
import { useUiTranslations } from "@/providers/locale-provider";

export function AgentCostLine({ agentId }: { agentId: string }) {
  const ui = useUiTranslations();
  const cost = useQuery({
    queryKey: ["costs", "agent", agentId],
    queryFn: () => getAgentCost(agentId),
    retry: false,
  });
  if (!cost.data) return null;
  return (
    <p className="text-sm text-muted-foreground">
      {ui("This month: {amount}", { amount: formatUsd(cost.data.cost_usd) })}
      {cost.data.unpriced_records > 0 ? ` ${ui("(partial: some usage has no price)")}` : ""}
      {" · "}
      <Link href="/settings/costs" className="underline underline-offset-2">
        {ui("All costs and usage policies")}
      </Link>
    </p>
  );
}
