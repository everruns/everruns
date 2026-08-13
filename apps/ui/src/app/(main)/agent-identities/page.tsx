"use client";

import Link from "next/link";
import { useMemo, useState } from "react";
import { Plus, UserRound } from "lucide-react";
import { Button } from "@/components/ui/button";
import { SearchInput } from "@/components/ui/search-input";
import { Badge } from "@/components/ui/badge";
import { EntityCard } from "@/components/ui/entity-card";
import { Skeleton } from "@/components/ui/skeleton";
import { useAgentIdentities } from "@/hooks/use-agent-identities";
import { usePageTitle } from "@/hooks";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  IconTile,
  PageControlStrip,
  SectionTabs,
  EmptyState,
  PageColumns,
  PageMain,
  PageRail,
  RailSection,
  PageFooter,
} from "@/components/layout";
import {
  getEntityNameClassName,
  getEntityStatusBadgeVariant,
  isArchivedStatus,
} from "@/lib/entity-lifecycle";
import { pluralize } from "@/lib/formatting";
import { cn } from "@/lib/utils";
import type { AgentIdentity } from "@/lib/api/types";

type StatusTab = "all" | "active" | "archived";

export function AgentIdentityCard({ identity }: { identity: AgentIdentity }) {
  return (
    <EntityCard
      icon={<IconTile size="md" icon={<UserRound />} />}
      title={identity.name}
      href={`/agent-identities/${identity.id}`}
      titleClassName={getEntityNameClassName(identity.status)}
      copyValue={identity.id}
      headerActions={
        <Badge variant={getEntityStatusBadgeVariant(identity.status)}>{identity.status}</Badge>
      }
    >
      <div className="space-y-3">
        {identity.description && (
          <p className="line-clamp-2 text-sm text-muted-foreground">{identity.description}</p>
        )}
        <p className="text-[13px] text-muted-foreground">
          {identity.locale || "No locale"} · {identity.timezone || "No timezone"}
        </p>
      </div>
    </EntityCard>
  );
}

export default function AgentIdentitiesPage() {
  usePageTitle("Agent Identities");
  // Fetch the full set (including archived) so the facet rail can show accurate counts.
  const { data: identities, isLoading, error } = useAgentIdentities({ includeArchived: true });

  const [search, setSearch] = useState("");
  const [statusTab, setStatusTab] = useState<StatusTab>("active");

  const counts = useMemo(() => {
    const list = identities ?? [];
    const archived = list.filter((i) => isArchivedStatus(i.status)).length;
    return { all: list.length, active: list.length - archived, archived };
  }, [identities]);

  const localeFacets = useMemo(() => {
    const tally = new Map<string, number>();
    for (const identity of identities ?? []) {
      if (identity.locale) tally.set(identity.locale, (tally.get(identity.locale) ?? 0) + 1);
    }
    return [...tally.entries()]
      .map(([locale, count]) => ({ locale, count }))
      .sort((a, b) => b.count - a.count)
      .slice(0, 8);
  }, [identities]);

  const timezoneFacets = useMemo(() => {
    const tally = new Map<string, number>();
    for (const identity of identities ?? []) {
      if (identity.timezone) tally.set(identity.timezone, (tally.get(identity.timezone) ?? 0) + 1);
    }
    return [...tally.entries()]
      .map(([timezone, count]) => ({ timezone, count }))
      .sort((a, b) => b.count - a.count)
      .slice(0, 8);
  }, [identities]);

  const filteredIdentities = useMemo(() => {
    const query = search.trim().toLowerCase();
    return (identities ?? []).filter((identity: AgentIdentity) => {
      if (statusTab === "active" && isArchivedStatus(identity.status)) return false;
      if (statusTab === "archived" && !isArchivedStatus(identity.status)) return false;
      if (!query) return true;
      const haystack = [identity.name, identity.description, identity.locale, identity.timezone]
        .filter(Boolean)
        .join(" ")
        .toLowerCase();
      return haystack.includes(query);
    });
  }, [identities, search, statusTab]);

  const statusItems = [
    { value: "all" as const, label: "All" },
    { value: "active" as const, label: "Active" },
    { value: "archived" as const, label: "Archived" },
  ];

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Agent Identities" }]} />

      <PageMasthead
        icon={<UserRound />}
        title="Agent Identities"
        badges={
          <Badge variant="outline" className="font-mono">
            {counts.all}
          </Badge>
        }
        description="Identities give agents a locale, timezone, and connections to run sessions as."
        meta={
          <>
            <span>{counts.active} active</span>
            <span>{counts.archived} archived</span>
          </>
        }
        actions={
          <Link href="/agent-identities/new">
            <Button variant="accent">
              <Plus className="size-4" />
              New identity
            </Button>
          </Link>
        }
      />

      {error && (
        <div className="border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">
          Error loading identities: {error.message}
        </div>
      )}

      <PageControlStrip className="flex flex-wrap items-center gap-3">
        <SearchInput
          placeholder="Search identities…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          containerClassName="w-64"
        />
        <div className="flex-1" />
        <SectionTabs
          value={statusTab}
          onValueChange={(v) => setStatusTab(v as StatusTab)}
          items={statusItems}
        />
      </PageControlStrip>

      <PageColumns>
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
              icon={<UserRound />}
              title={
                search || statusTab !== "active"
                  ? "No identities match your filters."
                  : "No agent identities yet."
              }
              action={
                !search &&
                statusTab === "active" && (
                  <Link href="/agent-identities/new">
                    <Button variant="accent">
                      <Plus className="size-4" />
                      Create your first identity
                    </Button>
                  </Link>
                )
              }
            />
          ) : (
            <div className="grid gap-4 md:grid-cols-2">
              {filteredIdentities.map((identity) => (
                <AgentIdentityCard key={identity.id} identity={identity} />
              ))}
            </div>
          )}
        </PageMain>

        <PageRail>
          <RailSection label="Status">
            <div className="flex flex-col gap-1.5 text-[13px]">
              {statusItems.map((item) => (
                <button
                  key={item.value}
                  type="button"
                  onClick={() => setStatusTab(item.value)}
                  className={cn(
                    "flex items-center justify-between transition-colors hover:text-foreground",
                    statusTab === item.value ? "text-foreground" : "text-muted-foreground",
                  )}
                >
                  <span>{item.label}</span>
                  <span className="text-muted-foreground">{counts[item.value]}</span>
                </button>
              ))}
            </div>
          </RailSection>

          {localeFacets.length > 0 && (
            <RailSection label="Locale">
              <div className="flex flex-col gap-1.5 text-[13px] text-muted-foreground">
                {localeFacets.map((facet) => (
                  <div key={facet.locale} className="flex items-center justify-between">
                    <span>{facet.locale}</span>
                    <span>{facet.count}</span>
                  </div>
                ))}
              </div>
            </RailSection>
          )}

          {timezoneFacets.length > 0 && (
            <RailSection label="Timezone">
              <div className="flex flex-col gap-1.5 text-[13px] text-muted-foreground">
                {timezoneFacets.map((facet) => (
                  <div key={facet.timezone} className="flex items-center justify-between">
                    <span className="truncate">{facet.timezone}</span>
                    <span>{facet.count}</span>
                  </div>
                ))}
              </div>
            </RailSection>
          )}
        </PageRail>
      </PageColumns>

      <PageFooter>
        <span>
          Showing {filteredIdentities.length} of {counts.all}{" "}
          {pluralize(counts.all, "identity", "identities")}
        </span>
        {counts.archived > 0 && statusTab !== "archived" && (
          <button
            type="button"
            onClick={() => setStatusTab("archived")}
            className="text-primary transition-colors hover:underline"
          >
            View archived →
          </button>
        )}
      </PageFooter>
    </PageContainer>
  );
}
