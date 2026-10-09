"use client";

import { HarnessDomainIcon } from "@/components/icons/facet-icons";
import { useCallback, useState, useMemo } from "react";
import {
  useCapabilities,
  useHarnessExamples,
  useHarnesses,
  useImportHarnessExample,
} from "@/hooks";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { LinkButton } from "@/components/ui/button";
import { SearchInput } from "@/components/ui/search-input";
import { ArrowRight, Plus, LayoutGrid, List as ListIcon } from "lucide-react";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { HarnessCard, HarnessExampleCard } from "@/components/harnesses";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  PageControlStrip,
  SectionTabs,
  EmptyState,
  PageMain,
} from "@/components/layout";
import { useLocale } from "@/providers/locale-provider";
import { isArchivedStatus } from "@/lib/entity-lifecycle";
import { Button } from "@/components/ui/button";
import { isHarnessDeprecated } from "@/lib/harness-deprecation";
import { cn } from "@/lib/utils";
import { resolveHarnessInheritance } from "@/lib/harness-inheritance";

const EXAMPLE_PREVIEW_LIMIT = 3;
type StatusTab = "all" | "active" | "archived";

export default function HarnessesPageClient() {
  const router = useRouter();
  const { locale } = useLocale();
  const { data: harnesses, isLoading, error } = useHarnesses({ includeArchived: true });
  const { data: allCapabilities } = useCapabilities({ includeRetired: true });
  const { data: examples, isLoading: examplesLoading, error: examplesError } = useHarnessExamples();
  const importExample = useImportHarnessExample();
  const [importingName, setImportingName] = useState<string | null>(null);

  const [showDeprecated, setShowDeprecated] = useState(false);
  const [search, setSearch] = useState("");
  const [statusTab, setStatusTab] = useState<StatusTab>("active");
  const [view, setView] = useState<"grid" | "list">("grid");

  const handleImport = useCallback(
    async (name: string) => {
      setImportingName(name);
      try {
        const harness = await importExample.mutateAsync(name);
        router.push(`/harnesses/${harness.id}`);
      } catch (err) {
        console.error("Failed to import harness example:", err);
      } finally {
        setImportingName(null);
      }
    },
    [importExample, router],
  );

  const counts = useMemo(() => {
    const list = (harnesses ?? []).filter((h) => showDeprecated || !isHarnessDeprecated(h));
    const archived = list.filter((h) => isArchivedStatus(h.status)).length;
    return { all: list.length, active: list.length - archived, archived };
  }, [harnesses, showDeprecated]);

  const harnessesById = useMemo(
    () => new Map((harnesses ?? []).map((harness) => [harness.id, harness])),
    [harnesses],
  );

  const filteredHarnesses = useMemo(() => {
    const query = search.trim().toLowerCase();
    return (harnesses ?? []).filter((harness) => {
      if (!showDeprecated && isHarnessDeprecated(harness)) return false;
      if (statusTab === "active" && isArchivedStatus(harness.status)) return false;
      if (statusTab === "archived" && !isArchivedStatus(harness.status)) return false;
      if (!query) return true;
      const haystack = [harness.display_name, harness.name, harness.description]
        .filter(Boolean)
        .join(" ")
        .toLowerCase();
      return haystack.includes(query);
    });
  }, [harnesses, search, statusTab, showDeprecated]);

  const previewExamples = examples?.slice(0, EXAMPLE_PREVIEW_LIMIT);
  const hasMoreExamples = (examples?.length ?? 0) > EXAMPLE_PREVIEW_LIMIT;

  const statusItems = [
    { value: "all" as const, label: "All" },
    { value: "active" as const, label: "Active" },
    { value: "archived" as const, label: "Archived" },
  ];

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Harnesses" }]} />

      <PageMasthead
        icon={<HarnessDomainIcon />}
        title="Harnesses"
        description="Shared runtime configuration — base prompt, capabilities, and policies sessions inherit."
        actions={
          <LinkButton variant="accent" href="/harnesses/new">
            <Plus className="size-4" />
            New harness
          </LinkButton>
        }
      />

      <PageControlStrip className="flex flex-wrap items-center gap-3">
        <SearchInput
          placeholder="Search harnesses…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          containerClassName="w-64"
        />
        <div className="flex-1" />
        <Button
          variant="ghost"
          size="sm"
          aria-pressed={showDeprecated}
          onClick={() => setShowDeprecated((shown) => !shown)}
        >
          {showDeprecated ? "Hide deprecated" : "Show deprecated"}
        </Button>
        <SectionTabs
          value={statusTab}
          onValueChange={(v) => setStatusTab(v as StatusTab)}
          items={statusItems.map((item) => ({ ...item, count: counts[item.value] }))}
        />
        <div className="flex border">
          <button
            type="button"
            aria-label="Grid view"
            aria-pressed={view === "grid"}
            onClick={() => setView("grid")}
            className={cn(
              "inline-flex size-8 items-center justify-center border-r",
              view === "grid"
                ? "bg-muted text-foreground"
                : "text-muted-foreground hover:bg-muted/50",
            )}
          >
            <LayoutGrid className="size-4" />
          </button>
          <button
            type="button"
            aria-label="List view"
            aria-pressed={view === "list"}
            onClick={() => setView("list")}
            className={cn(
              "inline-flex size-8 items-center justify-center",
              view === "list"
                ? "bg-muted text-foreground"
                : "text-muted-foreground hover:bg-muted/50",
            )}
          >
            <ListIcon className="size-4" />
          </button>
        </div>
      </PageControlStrip>

      <PageMain>
        <QueryStateWrapper
          isLoading={isLoading}
          error={error}
          data={filteredHarnesses}
          errorMessagePrefix="Failed to load harnesses"
          emptyState={
            <EmptyState
              icon={<HarnessDomainIcon />}
              title={
                search || statusTab !== "active"
                  ? "No harnesses match your filters."
                  : "No harnesses yet"
              }
              action={
                !search &&
                statusTab === "active" && (
                  <LinkButton variant="accent" href="/harnesses/new">
                    <Plus className="size-4" />
                    Create your first harness
                  </LinkButton>
                )
              }
            />
          }
        >
          {(items) => (
            <div className={cn("grid gap-4", view === "grid" ? "md:grid-cols-2" : "grid-cols-1")}>
              {items.map((harness) => (
                <HarnessCard
                  key={harness.id}
                  harness={harness}
                  allCapabilities={allCapabilities}
                  showEditButton
                  inheritance={resolveHarnessInheritance(harness, harnessesById)}
                />
              ))}
            </div>
          )}
        </QueryStateWrapper>
      </PageMain>

      {/* Example harnesses — a secondary discovery section below the primary frame. */}
      <section>
        <div className="mb-4 flex items-center justify-between">
          <h2 className="text-lg font-semibold tracking-tight">Example harnesses</h2>
          {hasMoreExamples && (
            <Link
              href="/harnesses/examples"
              className="inline-flex items-center gap-1 text-[13px] text-muted-foreground hover:text-foreground"
            >
              All examples
              <ArrowRight className="size-3.5" />
            </Link>
          )}
        </div>
        <QueryStateWrapper
          isLoading={examplesLoading}
          error={examplesError}
          data={previewExamples}
          errorMessagePrefix="Failed to load examples"
          emptyState={
            <div className="py-8 text-center">
              <p className="text-muted-foreground">No examples available</p>
            </div>
          }
        >
          {(items) => (
            <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
              {items.map((example, index) => (
                <HarnessExampleCard
                  key={example.name ?? `example-${index}`}
                  example={example}
                  allCapabilities={allCapabilities}
                  onImport={handleImport}
                  adopting={importingName === example.name}
                  preview
                />
              ))}
            </div>
          )}
        </QueryStateWrapper>
      </section>
    </PageContainer>
  );
}
