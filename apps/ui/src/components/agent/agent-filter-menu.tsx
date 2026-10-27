"use client";

import { useLocale as useDisplayLocale } from "@/providers/locale-provider";
import { useUiTranslations } from "@/providers/locale-provider";
import { useMemo, useState } from "react";
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
import { getDisplayName } from "@/lib/entity-lifecycle";

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
export function AgentFilterMenu({ value, onValueChange, className }: AgentFilterMenuProps) {
  const { displayLocale } = useDisplayLocale();
  const ui = useUiTranslations();
  const { data: agents = [] } = useAgents();
  const [open, setOpen] = useState(false);

  const agentMap = useMemo(() => {
    return new Map<string, Agent>(agents.map((a) => [a.id, a]));
  }, [agents]);

  const selected = value ? agentMap.get(value) : undefined;
  const displayValue = selected ? getDisplayName(selected, displayLocale) : "All agents";

  const handleValueChange = (newValue: string) => {
    onValueChange(newValue);
    setOpen(false);
  };

  return (
    <DropdownMenu open={open} onOpenChange={setOpen}>
      <DropdownMenuTrigger className={cn(buttonVariants({ variant: "outline" }), className)}>
        {displayValue}
        <ChevronDown className="h-4 w-4 ml-2 opacity-50" />
      </DropdownMenuTrigger>
      <DropdownMenuPositioner align="end">
        <DropdownMenuContent className="w-[200px]">
          <DropdownMenuRadioGroup value={value} onValueChange={handleValueChange}>
            <DropdownMenuRadioItem value="">{ui("All agents")}</DropdownMenuRadioItem>
            {agents.map((agent) => (
              <DropdownMenuRadioItem key={agent.id} value={agent.id}>
                {getDisplayName(agent, displayLocale)}
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuContent>
      </DropdownMenuPositioner>
    </DropdownMenu>
  );
}
