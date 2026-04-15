"use client";

import { useState } from "react";
import { useHarnesses, useCapabilities } from "@/hooks";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Plus } from "lucide-react";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { HarnessCard } from "@/components/harnesses";
import { ArchiveFilter } from "@/components/archive-filter";

export default function HarnessesPageClient() {
  const [showArchived, setShowArchived] = useState(false);
  const { data: harnesses, isLoading, error } = useHarnesses({ includeArchived: showArchived });
  const { data: allCapabilities } = useCapabilities();

  return (
    <div className="container mx-auto p-6">
      <div className="flex items-center justify-between mb-6">
        <h1 className="text-2xl font-bold">Harnesses</h1>
        <div className="flex items-center gap-2">
          <ArchiveFilter showArchived={showArchived} onShowArchivedChange={setShowArchived} />
          <Link href="/harnesses/new">
            <Button variant="accent">
              <Plus className="w-4 h-4 mr-2" />
              New Harness
            </Button>
          </Link>
        </div>
      </div>

      <QueryStateWrapper
        isLoading={isLoading}
        error={error}
        data={harnesses}
        errorMessagePrefix="Failed to load harnesses"
        emptyState={
          <div className="text-center py-12">
            <p className="text-muted-foreground mb-4">No harnesses yet</p>
            <Link href="/harnesses/new">
              <Button>
                <Plus className="w-4 h-4 mr-2" />
                Create your first harness
              </Button>
            </Link>
          </div>
        }
      >
        {(items) => (
          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
            {items.map((harness) => (
              <HarnessCard
                key={harness.id}
                harness={harness}
                allCapabilities={allCapabilities}
                showEditButton
              />
            ))}
          </div>
        )}
      </QueryStateWrapper>
    </div>
  );
}
