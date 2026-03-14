"use client";

import { useState } from "react";
import { useHarnesses, useCapabilities } from "@/hooks";
import Link from "next/link";
import { Button, buttonVariants } from "@/components/ui/button";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuPositioner,
  DropdownMenuContent,
  DropdownMenuCheckboxItem,
  DropdownMenuGroup,
  DropdownMenuLabel,
} from "@/components/ui/dropdown-menu";
import { Skeleton } from "@/components/ui/skeleton";
import { Filter, Plus } from "lucide-react";
import { cn } from "@/lib/utils";
import { HarnessCard } from "@/components/harnesses";

export default function HarnessesPage() {
  const [showArchived, setShowArchived] = useState(false);
  const { data: harnesses, isLoading, error } = useHarnesses({ includeArchived: showArchived });
  const { data: allCapabilities } = useCapabilities();

  if (error) {
    return (
      <div className="container mx-auto p-6">
        <div className="text-red-500">Error loading harnesses: {error.message}</div>
      </div>
    );
  }

  return (
    <div className="container mx-auto p-6">
      <div className="flex items-center justify-between mb-6">
        <h1 className="text-2xl font-bold">Harnesses</h1>
        <div className="flex items-center gap-2">
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
                    onCheckedChange={setShowArchived}
                  >
                    Show archived harnesses
                  </DropdownMenuCheckboxItem>
                </DropdownMenuGroup>
              </DropdownMenuContent>
            </DropdownMenuPositioner>
          </DropdownMenu>
          <Link href="/harnesses/new">
            <Button variant="accent">
              <Plus className="w-4 h-4 mr-2" />
              New Harness
            </Button>
          </Link>
        </div>
      </div>

      {isLoading ? (
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {[...Array(6)].map((_, i) => (
            <Card key={i}>
              <CardHeader>
                <Skeleton className="h-6 w-3/4" />
              </CardHeader>
              <CardContent>
                <Skeleton className="h-4 w-full mb-2" />
                <Skeleton className="h-4 w-2/3" />
              </CardContent>
            </Card>
          ))}
        </div>
      ) : harnesses?.length === 0 ? (
        <div className="text-center py-12">
          <p className="text-muted-foreground mb-4">No harnesses yet</p>
          <Link href="/harnesses/new">
            <Button>
              <Plus className="w-4 h-4 mr-2" />
              Create your first harness
            </Button>
          </Link>
        </div>
      ) : (
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {harnesses?.map((harness) => (
            <HarnessCard
              key={harness.id}
              harness={harness}
              allCapabilities={allCapabilities}
              showEditButton
            />
          ))}
        </div>
      )}
    </div>
  );
}
