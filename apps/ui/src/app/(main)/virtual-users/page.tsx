"use client";

import { useLocale as useFormattingLocale } from "@/providers/locale-provider";
import { useUiTranslations } from "@/providers/locale-provider";
import { useChatPageFilters } from "@/components/chat-mode/page-view-state";

import { VirtualUserIcon } from "@/components/icons/facet-icons";
import { EntityStatus } from "@/components/ui/entity-status";
import { useMemo, useState } from "react";
import { Plus } from "lucide-react";
import { Button, LinkButton } from "@/components/ui/button";
import { SearchInput } from "@/components/ui/search-input";
import { Badge } from "@/components/ui/badge";
import { EntityCard, EntityCardDescription } from "@/components/ui/entity-card";
import { Skeleton } from "@/components/ui/skeleton";
import { useVirtualUsers } from "@/hooks/use-virtual-users";
import { usePageTitle } from "@/hooks";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  IconTile,
  PageControlStrip,
  SectionTabs,
  EmptyState,
  PageMain,
  PageFooter,
} from "@/components/layout";
import { getEntityNameClassName, isArchivedStatus } from "@/lib/entity-lifecycle";
import { pluralize } from "@/lib/formatting";
import type { VirtualUser } from "@/lib/api/types";

type StatusTab = "all" | "active" | "archived";

export function VirtualUserCard({ identity }: { identity: VirtualUser }) {
  const ui = useUiTranslations();
  return (
    <EntityCard
      icon={<IconTile size="md" icon={<VirtualUserIcon />} />}
      title={identity.name}
      href={`/virtual-users/${identity.id}`}
      titleClassName={getEntityNameClassName(identity.status)}
      headerActions={<EntityStatus status={identity.status} />}
    >
      <div className="space-y-3">
        {identity.description && (
          <EntityCardDescription>{identity.description}</EntityCardDescription>
        )}
        <Badge variant="outline">
          {identity.usage === "end_user" ? ui("End user") : ui("Service")}
        </Badge>
        <p className="text-[13px] text-muted-foreground">
          {identity.locale || ui("No locale")} · {identity.timezone || ui("No timezone")}
        </p>
      </div>
    </EntityCard>
  );
}

export default function VirtualUsersPage() {
  const { displayLocale } = useFormattingLocale();
  const ui = useUiTranslations();
  usePageTitle(ui("Virtual Users"));
  const [search, setSearch] = useState("");
  const [usage, setUsage] = useState("all");
  const [statusTab, setStatusTab] = useState<StatusTab>("active");
  useChatPageFilters({ search, status: statusTab, usage });
  const {
    data: identities,
    isLoading,
    error,
    hasNextPage,
    fetchNextPage,
    isFetchingNextPage,
    total,
  } = useVirtualUsers({
    includeArchived: true,
    usage: usage === "all" ? undefined : (usage as "end_user" | "service"),
    search: search || undefined,
  });

  const counts = useMemo(() => {
    const list = identities ?? [];
    const archived = list.filter((i) => isArchivedStatus(i.status)).length;
    return { all: list.length, active: list.length - archived, archived };
  }, [identities]);

  const filteredIdentities = useMemo(() => {
    const query = search.trim().toLowerCase();
    return (identities ?? []).filter((identity: VirtualUser) => {
      if (usage !== "all" && identity.usage !== usage) return false;
      if (statusTab === "active" && isArchivedStatus(identity.status)) return false;
      if (statusTab === "archived" && !isArchivedStatus(identity.status)) return false;
      if (!query) return true;
      const haystack = [identity.name, identity.description, identity.locale, identity.timezone]
        .filter(Boolean)
        .join(" ")
        .toLowerCase();
      return haystack.includes(query);
    });
  }, [identities, search, statusTab, usage]);

  const statusItems = [
    { value: "all" as const, label: ui("All") },
    { value: "active" as const, label: ui("Active") },
    { value: "archived" as const, label: ui("Archived") },
  ];

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: ui("Virtual Users") }]} />

      <PageMasthead
        icon={<VirtualUserIcon />}
        title={ui("Virtual Users")}
        description={ui("Profiles and connections for people using agents and service accounts.")}
        actions={
          <LinkButton variant="accent" href="/virtual-users/new">
            <Plus className="size-4" />
            {ui("New virtual user")}
          </LinkButton>
        }
      />

      {error && (
        <div className="border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">
          {ui("Error loading virtual users:")} {error.message}
        </div>
      )}

      <PageControlStrip className="flex flex-wrap items-center gap-3">
        <SearchInput
          placeholder={ui("Search virtual users…")}
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          containerClassName="w-64"
        />
        <SectionTabs
          value={usage}
          onValueChange={setUsage}
          items={[
            { value: "all", label: ui("All usages") },
            { value: "end_user", label: ui("End users") },
            { value: "service", label: ui("Services") },
          ]}
        />
        <div className="flex-1" />
        <SectionTabs
          value={statusTab}
          onValueChange={(v) => setStatusTab(v as StatusTab)}
          items={statusItems}
        />
      </PageControlStrip>

      <PageMain>
        {isLoading ? (
          <div className="grid gap-4 md:grid-cols-2">
            {[...Array(4)].map((_, i) => (
              <div key={i} className="border bg-card p-4">
                <Skeleton className="mb-3 h-6 w-1/2" />
                <Skeleton className="h-4 w-full" />
              </div>
            ))}
          </div>
        ) : filteredIdentities.length === 0 ? (
          <EmptyState
            icon={<VirtualUserIcon />}
            title={
              search || statusTab !== "active"
                ? ui("No virtual users match your filters.")
                : ui("No virtual users yet.")
            }
            action={
              !search &&
              statusTab === "active" && (
                <LinkButton variant="accent" href="/virtual-users/new">
                  <Plus className="size-4" />
                  {ui("Create your first virtual user")}
                </LinkButton>
              )
            }
          />
        ) : (
          <div className="grid gap-4 md:grid-cols-2">
            {filteredIdentities.map((identity) => (
              <VirtualUserCard key={identity.id} identity={identity} />
            ))}
          </div>
        )}
        {hasNextPage && (
          <Button variant="outline" disabled={isFetchingNextPage} onClick={() => fetchNextPage()}>
            {ui("Load more")}
          </Button>
        )}
      </PageMain>

      <PageFooter>
        <span>
          {ui("Showing")} {filteredIdentities.length} {ui("of")} {counts.all}{" "}
          {pluralize(counts.all, "virtual user", "virtual users", displayLocale)}
        </span>
        {counts.archived > 0 && statusTab !== "archived" && (
          <button
            type="button"
            onClick={() => setStatusTab("archived")}
            className="text-primary transition-colors hover:underline"
          >
            {ui("View archived →")}
          </button>
        )}
      </PageFooter>
    </PageContainer>
  );
}
