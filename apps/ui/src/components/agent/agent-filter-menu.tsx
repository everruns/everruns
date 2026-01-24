"use client";

import { useMemo } from "react";
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuPositioner,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
} from "@/components/ui/dropdown-menu";
import { buttonVariants } from "@/components/ui/button";
import { useAgents } from "@/hooks";
import { ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";
import type { Agent } from "@/lib/api/types";

interface AgentFilterMenuProps {
  /** Selected agent ID (empty string = all) */
  value: string;
  /** Callback when selection changes */
  onValueChange: (agentId: string) => void;
  /** Additional className for trigger button */
  className?: string;
}

/**
 * Agent filter dropdown menu for filtering lists by agent.
 * Uses menu pattern (better for filters) instead of form select.
 */
export function AgentFilterMenu({
  value,
  onValueChange,
  className,
}: AgentFilterMenuProps) {
  const { data: agents = [] } = useAgents();

  const agentMap = useMemo(() => {
    return new Map<string, Agent>(agents.map((a) => [a.id, a]));
  }, [agents]);

  const displayValue = value ? agentMap.get(value)?.name : "All agents";

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        className={cn(buttonVariants({ variant: "outline" }), className)}
      >
        {displayValue}
        <ChevronDown className="h-4 w-4 ml-2 opacity-50" />
      </DropdownMenuTrigger>
      <DropdownMenuPositioner align="end">
        <DropdownMenuContent className="w-[200px]">
          <DropdownMenuRadioGroup value={value} onValueChange={onValueChange}>
            <DropdownMenuRadioItem value="">
              All agents
            </DropdownMenuRadioItem>
            {agents.map((agent) => (
              <DropdownMenuRadioItem key={agent.id} value={agent.id}>
                {agent.name}
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuContent>
      </DropdownMenuPositioner>
    </DropdownMenu>
  );
}
