"use client";

import Link from "next/link";
import { Plus } from "lucide-react";
import { useObservers, usePageTitle } from "@/hooks";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { ExperimentalPageBadge } from "@/components/ui/experimental-badge";
import { useFeatureFlag } from "@/providers/feature-flags-provider";
import { getEntityStatusBadgeVariant } from "@/lib/entity-lifecycle";
import { pluralize } from "@/lib/formatting";
import type { Observer } from "@/lib/api/types";

function ObserverCard({ observer }: { observer: Observer }) {
  return (
    <Link href={`/observers/${observer.id}`}>
      <Card className="h-full cursor-pointer transition-colors hover:border-primary/50">
        <CardHeader className="pb-2">
          <div className="flex items-center justify-between">
            <CardTitle className="truncate text-base">{observer.name}</CardTitle>
            <Badge variant={getEntityStatusBadgeVariant(observer.status)}>{observer.status}</Badge>
          </div>
        </CardHeader>
        <CardContent className="space-y-2">
          {observer.description && (
            <p className="line-clamp-2 text-sm text-muted-foreground">{observer.description}</p>
          )}
          <div className="flex items-center gap-4 text-xs text-muted-foreground">
            <span>{Math.round(observer.sampling_rate * 100)}% sampled</span>
            <span>
              {observer.scorers.length} {pluralize(observer.scorers.length, "scorer")}
            </span>
          </div>
        </CardContent>
      </Card>
    </Link>
  );
}

export default function ObserversPageClient() {
  usePageTitle("Observers");
  const observersEnabled = useFeatureFlag("observers");
  const {
    data: observers,
    isLoading,
    error,
  } = useObservers({
    includeArchived: false,
    enabled: observersEnabled,
  });

  if (!observersEnabled) {
    return (
      <div className="flex h-full items-center justify-center">
        <div className="text-center text-muted-foreground">
          <p className="text-lg font-medium">Observers is not enabled</p>
          <p className="text-sm">This feature is currently disabled.</p>
        </div>
      </div>
    );
  }

  return (
    <div className="container mx-auto space-y-6 p-6">
      <div className="flex items-center justify-between">
        <h1 className="flex items-center gap-3 text-2xl font-bold">
          Observers
          <ExperimentalPageBadge />
        </h1>
        <Link href="/observers/new">
          <Button variant="accent">
            <Plus className="mr-2 h-4 w-4" />
            New Observer
          </Button>
        </Link>
      </div>

      <QueryStateWrapper
        isLoading={isLoading}
        error={error}
        data={observers}
        errorMessagePrefix="Failed to load observers"
        emptyState={
          <div className="py-12 text-center">
            <p className="mb-4 text-muted-foreground">No observers yet</p>
            <Link href="/observers/new">
              <Button>
                <Plus className="mr-2 h-4 w-4" />
                Create observer
              </Button>
            </Link>
          </div>
        }
      >
        {(items) => (
          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
            {items.map((observer) => (
              <ObserverCard key={observer.id} observer={observer} />
            ))}
          </div>
        )}
      </QueryStateWrapper>
    </div>
  );
}
