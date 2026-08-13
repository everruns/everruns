"use client";

import { useMemo, useState } from "react";
import {
  useCapabilities,
  useDeclarativeCapabilities,
  useDeleteDeclarativeCapability,
  usePageTitle,
} from "@/hooks";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { SearchInput } from "@/components/ui/search-input";
import { Skeleton } from "@/components/ui/skeleton";
import Link from "next/link";
import { CircleOff, Bot, Layers, Plus, Pencil } from "lucide-react";
import { EntityCard } from "@/components/ui/entity-card";
import { EntityIdentity } from "@/components/ui/entity-identity";
import type { Capability, CapabilityStatus, DeclarativeCapability } from "@/lib/api/types";
import { CapabilityIcon } from "@/lib/capability-icons";
import { registryDomainIcons } from "@/lib/registry-navigation";
import {
  localizedCapabilityDescription,
  localizedCapabilityName,
} from "@/lib/capability-localization";
import { useLocale } from "@/providers/locale-provider";
import { InlineStreamdownMessage } from "@/components/chat/streamdown-message";
import { getCapabilityStatusBadgeVariant } from "@/lib/status-utils";
import { formatCountLabel, pluralize } from "@/lib/formatting";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  PageControlStrip,
  SectionTabs,
  IconTile,
  PageColumns,
  PageMain,
  PageRail,
  RailSection,
  PageFooter,
} from "@/components/layout";
import { cn } from "@/lib/utils";

const UNCATEGORIZED = "Uncategorized";
type StatusTab = "all" | CapabilityStatus;

const CapabilitiesIcon = registryDomainIcons.capabilities;

function getStatusLabel(status: CapabilityStatus): string {
  switch (status) {
    case "available":
      return "Available";
    case "coming_soon":
      return "Coming Soon";
    case "deprecated":
      return "Deprecated";
  }
}

function DeclarativeCapabilityRow({ capability }: { capability: DeclarativeCapability }) {
  const { locale } = useLocale();
  const deleteCapability = useDeleteDeclarativeCapability();
  return (
    <div className="flex items-center justify-between gap-3 border p-3">
      <div className="min-w-0 flex-1">
        <div className="font-medium">
          <EntityIdentity value={capability.capability_id}>
            <Link href={`/capabilities/declarative/${capability.id}`} className="hover:underline">
              {capability.display_name ?? localizedCapabilityName(capability, locale)}
            </Link>
          </EntityIdentity>
        </div>
        <p className="text-sm text-muted-foreground line-clamp-1">
          {localizedCapabilityDescription(capability, locale)}
        </p>
      </div>
      <div className="flex items-center gap-2 shrink-0">
        <Link href={`/capabilities/declarative/${capability.id}`}>
          <Button type="button" variant="outline" size="sm">
            <Pencil className="mr-1.5 h-3.5 w-3.5" />
            Edit
          </Button>
        </Link>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => deleteCapability.mutate(capability.id)}
          disabled={deleteCapability.isPending}
        >
          Archive
        </Button>
      </div>
    </div>
  );
}

function CapabilityStats({ capability }: { capability: Capability }) {
  const agents = capability.agent_count ?? 0;
  const harnesses = capability.harness_count ?? 0;
  return (
    <div className="flex items-center gap-3 text-xs text-muted-foreground">
      <span className="inline-flex items-center gap-1">
        <Bot className="h-3.5 w-3.5" />
        {formatCountLabel(agents, "agent")}
      </span>
      <span className="inline-flex items-center gap-1">
        <Layers className="h-3.5 w-3.5" />
        {formatCountLabel(harnesses, "harness", "harnesses")}
      </span>
    </div>
  );
}

function CapabilityCard({ capability }: { capability: Capability }) {
  const { locale } = useLocale();
  return (
    <EntityCard
      className="h-full"
      icon={<IconTile size="md" icon={<CapabilityIcon icon={capability.icon} />} />}
      title={localizedCapabilityName(capability, locale)}
      href={`/capabilities/${capability.id}`}
      copyValue={capability.id}
      headerActions={
        <Badge variant={getCapabilityStatusBadgeVariant(capability.status)}>
          {getStatusLabel(capability.status)}
        </Badge>
      }
    >
      <div className="text-sm text-muted-foreground mb-3 line-clamp-3">
        <InlineStreamdownMessage>
          {localizedCapabilityDescription(capability, locale)}
        </InlineStreamdownMessage>
      </div>
      <div className="flex items-center justify-between gap-2 flex-wrap">
        {capability.category ? (
          <Badge variant="outline" className="text-xs">
            {capability.category}
          </Badge>
        ) : (
          <span />
        )}
        <CapabilityStats capability={capability} />
      </div>
    </EntityCard>
  );
}

function matchesLocalizations(capability: Pick<Capability, "localizations">, q: string): boolean {
  return Object.values(capability.localizations ?? {}).some(
    (loc) =>
      (loc.name?.toLowerCase().includes(q) ?? false) ||
      (loc.description?.toLowerCase().includes(q) ?? false),
  );
}

function matches(capability: Capability, query: string): boolean {
  if (!query) return true;
  const q = query.toLowerCase();
  return (
    capability.name.toLowerCase().includes(q) ||
    capability.description.toLowerCase().includes(q) ||
    capability.id.toLowerCase().includes(q) ||
    (capability.category?.toLowerCase().includes(q) ?? false) ||
    matchesLocalizations(capability, q)
  );
}

function matchesDeclarative(capability: DeclarativeCapability, query: string): boolean {
  if (!query) return true;
  const q = query.toLowerCase();
  return (
    capability.name.toLowerCase().includes(q) ||
    (capability.display_name?.toLowerCase().includes(q) ?? false) ||
    capability.description.toLowerCase().includes(q) ||
    capability.id.toLowerCase().includes(q) ||
    capability.capability_id.toLowerCase().includes(q)
  );
}

export default function CapabilitiesPage() {
  usePageTitle("Capabilities");
  const { data: capabilities, isLoading, error } = useCapabilities();
  const { data: declarativeCapabilities } = useDeclarativeCapabilities();
  const [searchQuery, setSearchQuery] = useState("");
  const [selectedCategory, setSelectedCategory] = useState<string | null>(null);
  const [statusTab, setStatusTab] = useState<StatusTab>("all");

  const { categories, categoryCounts } = useMemo(() => {
    const counts: Record<string, number> = {};
    for (const cap of capabilities ?? []) {
      const key = cap.category || UNCATEGORIZED;
      counts[key] = (counts[key] ?? 0) + 1;
    }
    const cats = Object.keys(counts).sort((a, b) => {
      if (a === UNCATEGORIZED) return 1;
      if (b === UNCATEGORIZED) return -1;
      return a.localeCompare(b);
    });
    return { categories: cats, categoryCounts: counts };
  }, [capabilities]);

  const statusCounts = useMemo(() => {
    const list = capabilities ?? [];
    return {
      all: list.length,
      available: list.filter((c) => c.status === "available").length,
      coming_soon: list.filter((c) => c.status === "coming_soon").length,
      deprecated: list.filter((c) => c.status === "deprecated").length,
    } satisfies Record<StatusTab, number>;
  }, [capabilities]);

  const filtered = useMemo(() => {
    return (capabilities ?? []).filter((cap) => {
      if (!matches(cap, searchQuery)) return false;
      if (statusTab !== "all" && cap.status !== statusTab) return false;
      if (selectedCategory) {
        const key = cap.category || UNCATEGORIZED;
        if (key !== selectedCategory) return false;
      }
      return true;
    });
  }, [capabilities, searchQuery, selectedCategory, statusTab]);

  const filteredDeclarative = useMemo(
    () =>
      (declarativeCapabilities ?? []).filter((capability) =>
        matchesDeclarative(capability, searchQuery),
      ),
    [declarativeCapabilities, searchQuery],
  );

  const grouped = useMemo(() => {
    const map = new Map<string, Capability[]>();
    for (const cap of filtered) {
      const key = cap.category || UNCATEGORIZED;
      if (!map.has(key)) map.set(key, []);
      map.get(key)!.push(cap);
    }
    return Array.from(map.entries()).sort(([a], [b]) => {
      if (a === UNCATEGORIZED) return 1;
      if (b === UNCATEGORIZED) return -1;
      return a.localeCompare(b);
    });
  }, [filtered]);

  const totalCount = capabilities?.length ?? 0;
  const filteredCount = filtered.length;
  const isFiltering = searchQuery.length > 0 || selectedCategory !== null || statusTab !== "all";

  const statusItems = [
    { value: "all" as const, label: "All" },
    { value: "available" as const, label: "Available" },
    { value: "coming_soon" as const, label: "Coming Soon" },
    { value: "deprecated" as const, label: "Deprecated" },
  ];

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Capabilities" }]} />

      <PageMasthead
        icon={<CapabilitiesIcon />}
        title="Capabilities"
        badges={
          <Badge variant="outline" className="font-mono">
            {totalCount}
          </Badge>
        }
        description="Building blocks — system prompt additions and tools agents and harnesses can use."
        meta={
          <>
            <span>{statusCounts.available} available</span>
            <span>{statusCounts.coming_soon} coming soon</span>
            <span>{statusCounts.deprecated} deprecated</span>
          </>
        }
        actions={
          <Link href="/capabilities/declarative/new">
            <Button variant="accent">
              <Plus className="size-4" />
              New Declarative
            </Button>
          </Link>
        }
      />

      {error && (
        <div className="border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">
          Error loading capabilities: {error.message}
        </div>
      )}

      <PageControlStrip className="flex flex-wrap items-center gap-3">
        <SearchInput
          placeholder="Search by name, description, ID, or category…"
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          containerClassName="w-72"
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
          {filteredDeclarative.length > 0 && (
            <Card>
              <CardHeader>
                <CardTitle className="text-base">Declarative Capabilities</CardTitle>
              </CardHeader>
              <CardContent className="space-y-2">
                {filteredDeclarative.map((capability) => (
                  <DeclarativeCapabilityRow key={capability.id} capability={capability} />
                ))}
              </CardContent>
            </Card>
          )}

          {isLoading ? (
            <div className="grid gap-4 md:grid-cols-2">
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
          ) : totalCount === 0 ? (
            <Card className="p-8 text-center">
              <CircleOff className="h-12 w-12 mx-auto text-muted-foreground mb-4" />
              <h3 className="text-lg font-medium mb-2">No capabilities available</h3>
              <p className="text-muted-foreground">
                Capabilities will appear here once they are configured.
              </p>
            </Card>
          ) : filteredCount === 0 ? (
            <Card className="p-8 text-center">
              <CircleOff className="h-12 w-12 mx-auto text-muted-foreground mb-4" />
              <h3 className="text-lg font-medium mb-2">No matches</h3>
              <p className="text-muted-foreground mb-4">
                No capabilities match the current search and filters.
              </p>
              <Button
                variant="outline"
                onClick={() => {
                  setSearchQuery("");
                  setSelectedCategory(null);
                  setStatusTab("all");
                }}
              >
                Clear filters
              </Button>
            </Card>
          ) : (
            <div className="space-y-8">
              {grouped.map(([category, caps]) => (
                <section key={category}>
                  <div className="flex items-center justify-between mb-3">
                    <h2 className="text-sm font-semibold uppercase tracking-wide text-muted-foreground">
                      {category}
                    </h2>
                    <span className="text-xs text-muted-foreground">
                      {formatCountLabel(caps.length, "capability", "capabilities")}
                    </span>
                  </div>
                  <div className="grid gap-4 md:grid-cols-2">
                    {caps.map((capability) => (
                      <CapabilityCard key={capability.id} capability={capability} />
                    ))}
                  </div>
                </section>
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
                  <span className="text-muted-foreground">{statusCounts[item.value]}</span>
                </button>
              ))}
            </div>
          </RailSection>

          {categories.length > 0 && (
            <RailSection label="Category">
              <div className="flex flex-col gap-1.5 text-[13px]">
                <button
                  type="button"
                  onClick={() => setSelectedCategory(null)}
                  aria-pressed={selectedCategory === null}
                  className={cn(
                    "flex items-center justify-between transition-colors hover:text-foreground",
                    selectedCategory === null ? "text-foreground" : "text-muted-foreground",
                  )}
                >
                  <span>All</span>
                  <span className="text-muted-foreground">
                    {Object.values(categoryCounts).reduce((a, b) => a + b, 0)}
                  </span>
                </button>
                {categories.map((category) => (
                  <button
                    key={category}
                    type="button"
                    onClick={() =>
                      setSelectedCategory(category === selectedCategory ? null : category)
                    }
                    aria-pressed={selectedCategory === category}
                    className={cn(
                      "flex items-center justify-between transition-colors hover:text-foreground",
                      selectedCategory === category ? "text-foreground" : "text-muted-foreground",
                    )}
                  >
                    <span>{category}</span>
                    <span className="text-muted-foreground">{categoryCounts[category] ?? 0}</span>
                  </button>
                ))}
              </div>
            </RailSection>
          )}
        </PageRail>
      </PageColumns>

      <PageFooter>
        <span>
          {isFiltering
            ? `Showing ${filteredCount} of ${totalCount} ${pluralize(totalCount, "capability", "capabilities")}`
            : `Showing ${formatCountLabel(totalCount, "capability", "capabilities")}`}
        </span>
        {isFiltering && (
          <button
            type="button"
            onClick={() => {
              setSearchQuery("");
              setSelectedCategory(null);
              setStatusTab("all");
            }}
            className="text-primary transition-colors hover:underline"
          >
            Clear filters →
          </button>
        )}
      </PageFooter>
    </PageContainer>
  );
}
