"use client";

import { Bot, Layers } from "lucide-react";
import type { SkillUsage } from "@/lib/api/types";

export function SkillUsageRow({ usage }: { usage: SkillUsage | undefined }) {
  const agents = usage?.agents ?? 0;
  const harnesses = usage?.harnesses ?? 0;
  const agentsLabel = `${agents} ${agents === 1 ? "agent" : "agents"}`;
  const harnessesLabel = `${harnesses} ${harnesses === 1 ? "harness" : "harnesses"}`;
  if (agents === 0 && harnesses === 0) {
    return <div className="text-xs text-muted-foreground">Not in use</div>;
  }
  return (
    <div className="flex items-center gap-3 text-xs text-muted-foreground">
      <span className="inline-flex items-center gap-1" aria-label={agentsLabel}>
        <Bot className="h-3.5 w-3.5" />
        {agentsLabel}
      </span>
      <span className="inline-flex items-center gap-1" aria-label={harnessesLabel}>
        <Layers className="h-3.5 w-3.5" />
        {harnessesLabel}
      </span>
    </div>
  );
}
