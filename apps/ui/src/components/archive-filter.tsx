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
}

export function ArchiveFilter({ showArchived, onShowArchivedChange }: ArchiveFilterProps) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger className={cn(buttonVariants({ variant: "outline" }), "gap-1.5")}>
        <Filter className="size-4" />
        Filter
        {showArchived && (
          <span className="bg-primary text-primary-foreground rounded-full px-1.5 text-xs">1</span>
        )}
      </DropdownMenuTrigger>
      <DropdownMenuPositioner align="end">
        <DropdownMenuContent className="w-56">
          <DropdownMenuGroup>
            <DropdownMenuLabel>Filters</DropdownMenuLabel>
            <DropdownMenuCheckboxItem checked={showArchived} onCheckedChange={onShowArchivedChange}>
              Show archived
            </DropdownMenuCheckboxItem>
          </DropdownMenuGroup>
        </DropdownMenuContent>
      </DropdownMenuPositioner>
    </DropdownMenu>
  );
}
