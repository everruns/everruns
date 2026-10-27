"use client";

import { useLocale as useFormattingLocale } from "@/providers/locale-provider";
import { useUiTranslations } from "@/providers/locale-provider";
import { useChatPageFilters } from "@/components/chat-mode/page-view-state";

import { ObserverIcon } from "@/components/icons/facet-icons";
import { EntityStatus } from "@/components/ui/entity-status";
import { useMemo, useState } from "react";
import { Plus } from "lucide-react";
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
  const { displayLocale } = useFormattingLocale();
  const ui = useUiTranslations();
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
          <span>
            {Math.round(observer.sampling_rate * 100)}
            {ui("% sampled")}
          </span>
          <span>
            {observer.scorers.length}{" "}
            {pluralize(observer.scorers.length, "scorer", undefined, displayLocale)}
          </span>
        </div>
      </div>
    </EntityCard>
  );
}

export default function ObserversPageClient() {
  const ui = useUiTranslations();
  usePageTitle(ui("Observers"));
  const observersEnabled = useFeatureFlag("observers");
  const [statusTab, setStatusTab] = useState<StatusTab>("active");
  useChatPageFilters({ status: statusTab });
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
    { value: "active" as const, label: ui("Active") },
    { value: "archived" as const, label: ui("Archived") },
  ];

  if (!observersEnabled) {
    return (
      <div className="flex h-full items-center justify-center">
        <div className="text-center text-muted-foreground">
          <p className="text-lg font-medium">{ui("Observers is not enabled")}</p>
          <p className="text-sm">{ui("This feature is currently disabled.")}</p>
        </div>
      </div>
    );
  }

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: ui("Observers") }]} />

      <PageMasthead
        icon={<ObserverIcon />}
        title={ui("Observers")}
        description={ui(
          "Score production sessions asynchronously with sampling rules and evaluators.",
        )}
        actions={
          <LinkButton variant="accent" href="/observers/new">
            <Plus className="size-4" />
            {ui("New Observer")}
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
              <p className="mb-4 text-muted-foreground">{ui("No observers yet")}</p>
              <LinkButton href="/observers/new">
                <Plus className="mr-2 size-4" />
                {ui("Create observer")}
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
