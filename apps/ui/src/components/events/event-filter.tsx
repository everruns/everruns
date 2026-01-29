"use client";

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
}

export function EventFilter({ hideDeltaEvents, onHideDeltaEventsChange }: EventFilterProps) {
  const activeFilterCount = hideDeltaEvents ? 1 : 0;

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        className={cn(buttonVariants({ variant: "outline", size: "sm" }), "gap-1.5")}
      >
        <Filter className="size-3.5" />
        Filter
        {activeFilterCount > 0 && (
          <span className="bg-primary text-primary-foreground rounded-full px-1.5 text-xs">
            {activeFilterCount}
          </span>
        )}
      </DropdownMenuTrigger>
      <DropdownMenuPositioner align="end">
        <DropdownMenuContent className="w-56">
          <DropdownMenuGroup>
            <DropdownMenuLabel>Event Filters</DropdownMenuLabel>
            <DropdownMenuCheckboxItem
              checked={hideDeltaEvents}
              onCheckedChange={onHideDeltaEventsChange}
            >
              Hide streaming events
            </DropdownMenuCheckboxItem>
          </DropdownMenuGroup>
        </DropdownMenuContent>
      </DropdownMenuPositioner>
    </DropdownMenu>
  );
}
