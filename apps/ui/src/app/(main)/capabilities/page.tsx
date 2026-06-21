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
import { CircleOff, Bot, Layers, X, Plus, Pencil } from "lucide-react";
import { CopyButton } from "@/components/ui/copy-button";
import type { Capability, CapabilityStatus, DeclarativeCapability } from "@/lib/api/types";
import { getCapabilityIcon } from "@/lib/capability-icons";
import {
  localizedCapabilityDescription,
  localizedCapabilityName,
} from "@/lib/capability-localization";
import { useLocale } from "@/providers/locale-provider";
import { InlineStreamdownMessage } from "@/components/chat/streamdown-message";
import { getCapabilityStatusBadgeVariant } from "@/lib/status-utils";
import { formatCountLabel } from "@/lib/formatting";
import { cn } from "@/lib/utils";

const UNCATEGORIZED = "Uncategorized";

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

function NewDeclarativeButton() {
  return (
    <Link href="/capabilities/declarative/new">
      <Button type="button" size="sm">
        <Plus className="mr-1.5 h-4 w-4" />
        New Declarative
      </Button>
    </Link>
  );
}

function DeclarativeCapabilityRow({ capability }: { capability: DeclarativeCapability }) {
  const { locale } = useLocale();
  const deleteCapability = useDeleteDeclarativeCapability();
  return (
    <div className="flex items-center justify-between gap-3 border p-3">
      <Link href={`/capabilities/declarative/${capability.id}`} className="min-w-0 flex-1 group">
        <div className="flex items-center gap-2">
          <p className="font-medium truncate group-hover:underline">
            {capability.display_name ?? localizedCapabilityName(capability, locale)}
          </p>
        </div>
        <p className="text-sm text-muted-foreground line-clamp-1">
          {localizedCapabilityDescription(capability, locale)}
        </p>
      </Link>
      <div className="flex items-center gap-2 shrink-0">
        <CopyButton value={capability.capability_id} />
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
  const IconComponent = getCapabilityIcon(capability.icon);

  return (
    <Link href={`/capabilities/${capability.id}`}>
      <Card className="h-full cursor-pointer bg-background transition-colors hover:bg-card">
        <CardHeader className="flex flex-row items-start justify-between space-y-0">
          <div className="flex items-center gap-3 min-w-0">
            <div className="border bg-muted p-2 shrink-0">
              <IconComponent className="icon-sharp h-5 w-5" />
            </div>
            <div className="flex items-center gap-2 min-w-0">
              <CardTitle className="text-lg truncate">
                {localizedCapabilityName(capability, locale)}
              </CardTitle>
              <CopyButton value={capability.id} />
            </div>
          </div>
          <Badge variant={getCapabilityStatusBadgeVariant(capability.status)}>
            {getStatusLabel(capability.status)}
          </Badge>
        </CardHeader>
        <CardContent>
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
        </CardContent>
      </Card>
    </Link>
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

function CategoryFilter({
  categories,
  selected,
  counts,
  onSelect,
}: {
  categories: string[];
  selected: string | null;
  counts: Record<string, number>;
  onSelect: (category: string | null) => void;
}) {
  if (categories.length === 0) return null;

  return (
    <div className="flex flex-wrap gap-2" role="group" aria-label="Filter by category">
      <button
        type="button"
        onClick={() => onSelect(null)}
        aria-pressed={selected === null}
        className={cn(
          "inline-flex items-center gap-1.5 border px-2.5 py-1 text-xs transition-colors",
          selected === null
            ? "bg-primary text-primary-foreground border-primary"
            : "bg-background hover:bg-muted",
        )}
      >
        All
        <span className="text-[10px] opacity-70">
          {Object.values(counts).reduce((a, b) => a + b, 0)}
        </span>
      </button>
      {categories.map((category) => (
        <button
          key={category}
          type="button"
          onClick={() => onSelect(category === selected ? null : category)}
          aria-pressed={selected === category}
          className={cn(
            "inline-flex items-center gap-1.5 border px-2.5 py-1 text-xs transition-colors",
            selected === category
              ? "bg-primary text-primary-foreground border-primary"
              : "bg-background hover:bg-muted",
          )}
        >
          {category}
          <span className="text-[10px] opacity-70">{counts[category] ?? 0}</span>
        </button>
      ))}
    </div>
  );
}

export default function CapabilitiesPage() {
  usePageTitle("Capabilities");
  const { data: capabilities, isLoading, error } = useCapabilities();
  const { data: declarativeCapabilities } = useDeclarativeCapabilities();
  const [searchQuery, setSearchQuery] = useState("");
  const [selectedCategory, setSelectedCategory] = useState<string | null>(null);

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

  const filtered = useMemo(() => {
    return (capabilities ?? []).filter((cap) => {
      if (!matches(cap, searchQuery)) return false;
      if (selectedCategory) {
        const key = cap.category || UNCATEGORIZED;
        if (key !== selectedCategory) return false;
      }
      return true;
    });
  }, [capabilities, searchQuery, selectedCategory]);

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

  if (error) {
    return (
      <div className="container mx-auto p-6">
        <div className="flex items-center justify-between mb-6">
          <h1 className="text-2xl font-bold">Capabilities</h1>
        </div>
        <div className="text-red-500">Error loading capabilities: {error.message}</div>
      </div>
    );
  }

  const totalCount = capabilities?.length ?? 0;
  const filteredCount = filtered.length;
  const isFiltering = searchQuery.length > 0 || selectedCategory !== null;

  return (
    <div className="container mx-auto p-6 space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">Capabilities</h1>
          <span className="text-sm text-muted-foreground">
            {isFiltering
              ? `${filteredCount} of ${totalCount}`
              : formatCountLabel(totalCount, "capability", "capabilities")}
          </span>
        </div>
        <NewDeclarativeButton />
      </div>

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

      {/* Search */}
      <SearchInput
        containerClassName="max-w-xl"
        className="pr-9"
        placeholder="Search by name, description, ID, or category..."
        value={searchQuery}
        onChange={(e) => setSearchQuery(e.target.value)}
      >
        {searchQuery && (
          <button
            type="button"
            onClick={() => setSearchQuery("")}
            className="absolute right-2 top-1/2 -translate-y-1/2 p-1 text-muted-foreground hover:text-foreground"
            aria-label="Clear search"
          >
            <X className="w-4 h-4" />
          </button>
        )}
      </SearchInput>

      {/* Category filter */}
      {!isLoading && (
        <CategoryFilter
          categories={categories}
          selected={selectedCategory}
          counts={categoryCounts}
          onSelect={setSelectedCategory}
        />
      )}

      {/* Loading skeleton */}
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
              <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
                {caps.map((capability) => (
                  <CapabilityCard key={capability.id} capability={capability} />
                ))}
              </div>
            </section>
          ))}
        </div>
      )}
    </div>
  );
}
