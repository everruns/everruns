"use client";

import { EntityStatus } from "@/components/ui/entity-status";
import { useMemo, useState } from "react";
import { Plus, Telescope } from "lucide-react";
import { useObservers, usePageTitle } from "@/hooks";
import { LinkButton } from "@/components/ui/button";
import { EntityCard, EntityCardDescription } from "@/components/ui/entity-card";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { useFeatureFlag } from "@/providers/feature-flags-provider";
import { isArchivedStatus } from "@/lib/entity-lifecycle";
import { pluralize } from "@/lib/formatting";
import type { Observer } from "@/lib/api/types";
import {
  PageBreadcrumb,
  PageContainer,
  PageControlStrip,
  PageMain,
  PageMasthead,
  SectionTabs,
} from "@/components/layout";

type StatusTab = "active" | "archived";

function ObserverCard({ observer }: { observer: Observer }) {
  return (
    <EntityCard
      className="h-full"
      title={observer.name}
      href={`/observers/${observer.id}`}
      headerActions={<EntityStatus status={observer.status} />}
    >
      <div className="space-y-2">
        {observer.description && (
          <EntityCardDescription>{observer.description}</EntityCardDescription>
        )}
        <div className="flex items-center gap-4 text-xs text-muted-foreground">
          <span>{Math.round(observer.sampling_rate * 100)}% sampled</span>
          <span>
            {observer.scorers.length} {pluralize(observer.scorers.length, "scorer")}
          </span>
        </div>
      </div>
    </EntityCard>
  );
}

export default function ObserversPageClient() {
  usePageTitle("Observers");
  const observersEnabled = useFeatureFlag("observers");
  const [statusTab, setStatusTab] = useState<StatusTab>("active");
  const {
    data: observers,
    isLoading,
    error,
  } = useObservers({
    includeArchived: true,
    enabled: observersEnabled,
  });
  const counts = useMemo(() => {
    const list = observers ?? [];
    const archived = list.filter((observer) => isArchivedStatus(observer.status)).length;
    return { all: list.length, active: list.length - archived, archived };
  }, [observers]);
  const filteredObservers = useMemo(
    () =>
      (observers ?? []).filter((observer) =>
        statusTab === "archived"
          ? isArchivedStatus(observer.status)
          : !isArchivedStatus(observer.status),
      ),
    [observers, statusTab],
  );
  const statusItems = [
    { value: "active" as const, label: "Active" },
    { value: "archived" as const, label: "Archived" },
  ];

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
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Observers" }]} />

      <PageMasthead
        icon={<Telescope />}
        title="Observers"
        description="Score production sessions asynchronously with sampling rules and evaluators."
        actions={
          <LinkButton variant="accent" href="/observers/new">
            <Plus className="size-4" />
            New Observer
          </LinkButton>
        }
      />

      <PageControlStrip className="flex justify-end">
        <SectionTabs
          value={statusTab}
          onValueChange={(value) => setStatusTab(value as StatusTab)}
          items={statusItems.map((item) => ({ ...item, count: counts[item.value] }))}
        />
      </PageControlStrip>

      <PageMain>
        <QueryStateWrapper
          isLoading={isLoading}
          error={error}
          data={filteredObservers}
          errorMessagePrefix="Failed to load observers"
          emptyState={
            <div className="py-12 text-center">
              <p className="mb-4 text-muted-foreground">No observers yet</p>
              <LinkButton href="/observers/new">
                <Plus className="mr-2 size-4" />
                Create observer
              </LinkButton>
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
      </PageMain>
    </PageContainer>
  );
}
