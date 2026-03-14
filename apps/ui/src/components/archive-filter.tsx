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

interface ArchiveFilterProps {
  showArchived: boolean;
  onShowArchivedChange: (show: boolean) => void;
  /** Label for the checkbox item, e.g. "Show archived agents" */
  label?: string;
}

export function ArchiveFilter({
  showArchived,
  onShowArchivedChange,
  label = "Show archived",
}: ArchiveFilterProps) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        className={cn(buttonVariants({ variant: "outline", size: "sm" }), "gap-1.5")}
      >
        <Filter className="size-3.5" />
        Filter
        {showArchived && (
          <span className="bg-primary text-primary-foreground rounded-full px-1.5 text-xs">
            1
          </span>
        )}
      </DropdownMenuTrigger>
      <DropdownMenuPositioner align="end">
        <DropdownMenuContent className="w-56">
          <DropdownMenuGroup>
            <DropdownMenuLabel>Filters</DropdownMenuLabel>
            <DropdownMenuCheckboxItem
              checked={showArchived}
              onCheckedChange={onShowArchivedChange}
            >
              {label}
            </DropdownMenuCheckboxItem>
          </DropdownMenuGroup>
        </DropdownMenuContent>
      </DropdownMenuPositioner>
    </DropdownMenu>
  );
}
