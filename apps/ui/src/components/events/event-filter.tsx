"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { Filter } from "lucide-react";
import { buttonVariants } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuPositioner,
  DropdownMenuContent,
  DropdownMenuCheckboxItem,
  DropdownMenuGroup,
  DropdownMenuLabel,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";

interface EventFilterProps {
  hideDeltaEvents: boolean;
  onHideDeltaEventsChange: (hide: boolean) => void;
  onlyLlmGeneration: boolean;
  onOnlyLlmGenerationChange: (only: boolean) => void;
}

export function EventFilter({
  hideDeltaEvents,
  onHideDeltaEventsChange,
  onlyLlmGeneration,
  onOnlyLlmGenerationChange,
}: EventFilterProps) {
  const ui = useUiTranslations();
  const activeFilterCount = (hideDeltaEvents ? 1 : 0) + (onlyLlmGeneration ? 1 : 0);

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        className={cn(buttonVariants({ variant: "outline", size: "sm" }), "gap-1.5")}
      >
        <Filter className="size-3.5" />
        {ui("Filter")}
        {activeFilterCount > 0 && (
          <span className="bg-primary text-primary-foreground rounded-full px-1.5 text-xs">
            {activeFilterCount}
          </span>
        )}
      </DropdownMenuTrigger>
      <DropdownMenuPositioner align="end">
        <DropdownMenuContent className="w-56">
          <DropdownMenuGroup>
            <DropdownMenuLabel>{ui("Event Filters")}</DropdownMenuLabel>
            <DropdownMenuCheckboxItem
              checked={hideDeltaEvents}
              onCheckedChange={onHideDeltaEventsChange}
            >
              {ui("Hide streaming events")}
            </DropdownMenuCheckboxItem>
            <DropdownMenuCheckboxItem
              checked={onlyLlmGeneration}
              onCheckedChange={onOnlyLlmGenerationChange}
            >
              {ui("Only llm.generation events")}
            </DropdownMenuCheckboxItem>
          </DropdownMenuGroup>
        </DropdownMenuContent>
      </DropdownMenuPositioner>
    </DropdownMenu>
  );
}
