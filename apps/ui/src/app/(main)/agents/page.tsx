"use client";

import { useRef, useState, useCallback, useMemo } from "react";
import {
  useAgents,
  useAgentExamples,
  useImportAgentExample,
  useCapabilities,
  useImportAgent,
  usePageTitle,
} from "@/hooks";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { NewAgentLink } from "@/components/agents/new-agent-link";
import { Button } from "@/components/ui/button";
import { SearchInput } from "@/components/ui/search-input";
import { Badge } from "@/components/ui/badge";
import { Plus, Upload, ArrowRight, Boxes, LayoutGrid, List as ListIcon } from "lucide-react";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { AgentCard, ExampleCard } from "@/components/agents";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  PageControlStrip,
  SectionTabs,
  EmptyState,
  PageColumns,
  PageMain,
  PageRail,
  RailSection,
  PageFooter,
} from "@/components/layout";
import { CapabilityIcon } from "@/lib/capability-icons";
import { localizedCapabilityName } from "@/lib/capability-localization";
import { useLocale } from "@/providers/locale-provider";
import { isArchivedStatus } from "@/lib/entity-lifecycle";
import { normalizeTags } from "@/lib/tags";
import { pluralize } from "@/lib/formatting";
import { cn } from "@/lib/utils";
import type { Agent } from "@/lib/api/types";

const EXAMPLE_PREVIEW_LIMIT = 6;
type StatusTab = "all" | "active" | "archived";

export default function AgentsPage() {
  usePageTitle("Agents");
  const { locale } = useLocale();
  const router = useRouter();
  // Fetch the full set (including archived) so the facet rail can show accurate counts.
  const { data: agents, isLoading, error } = useAgents({ includeArchived: true });
  const { data: allCapabilities } = useCapabilities();
  const { data: examples, isLoading: examplesLoading, error: examplesError } = useAgentExamples();
  const importAgent = useImportAgent();
  const importExample = useImportAgentExample();
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [importError, setImportError] = useState<string | null>(null);
  const [importingName, setImportingName] = useState<string | null>(null);

  const [search, setSearch] = useState("");
  const [statusTab, setStatusTab] = useState<StatusTab>("active");
  const [view, setView] = useState<"grid" | "list">("grid");

  const handleImportClick = useCallback(() => {
    fileInputRef.current?.click();
  }, []);

  const handleFileChange = useCallback(
    async (e: React.ChangeEvent<HTMLInputElement>) => {
      const file = e.target.files?.[0];
      if (!file) return;

      setImportError(null);

      try {
        const content = await file.text();
        const agent = await importAgent.mutateAsync(content);
        router.push(`/agents/${agent.id}`);
      } catch (err) {
        console.error("Failed to import agent:", err);
        setImportError(err instanceof Error ? err.message : "Failed to import agent");
      }

      if (fileInputRef.current) {
        fileInputRef.current.value = "";
      }
    },
    [importAgent, router],
  );

  const handleImport = useCallback(
    async (name: string) => {
      setImportingName(name);
      try {
        const agent = await importExample.mutateAsync(name);
        router.push(`/agents/${agent.id}`);
      } catch (err) {
        console.error("Failed to import example:", err);
      } finally {
        setImportingName(null);
      }
    },
    [importExample, router],
  );

  const counts = useMemo(() => {
    const list = agents ?? [];
    const archived = list.filter((a) => isArchivedStatus(a.status)).length;
    return { all: list.length, active: list.length - archived, archived };
  }, [agents]);

  // Capability usage counts across all agents, for the rail facet.
  const capabilityFacets = useMemo(() => {
    const tally = new Map<string, number>();
    for (const agent of agents ?? []) {
      for (const cap of agent.capabilities ?? []) {
        tally.set(cap.ref, (tally.get(cap.ref) ?? 0) + 1);
      }
    }
    return [...tally.entries()]
      .map(([ref, count]) => {
        const cap = allCapabilities?.find((c) => c.id === ref);
        const icon = cap?.icon;
        return {
          ref,
          count,
          name: cap ? localizedCapabilityName(cap, locale) : ref,
          icon: ({ className }: { className?: string }) => (
            <CapabilityIcon icon={icon} className={className} />
          ),
        };
      })
      .sort((a, b) => b.count - a.count)
      .slice(0, 6);
  }, [agents, allCapabilities, locale]);

  const tagFacets = useMemo(() => {
    const tags = new Set<string>();
    for (const agent of agents ?? []) {
      for (const tag of normalizeTags(agent.tags)) tags.add(tag);
    }
    return [...tags].slice(0, 12);
  }, [agents]);

  const filteredAgents = useMemo(() => {
    const query = search.trim().toLowerCase();
    return (agents ?? []).filter((agent: Agent) => {
      if (statusTab === "active" && isArchivedStatus(agent.status)) return false;
      if (statusTab === "archived" && !isArchivedStatus(agent.status)) return false;
      if (!query) return true;
      const haystack = [agent.display_name, agent.name, agent.description]
        .filter(Boolean)
        .join(" ")
        .toLowerCase();
      return haystack.includes(query);
    });
  }, [agents, search, statusTab]);

  const previewExamples = examples?.slice(0, EXAMPLE_PREVIEW_LIMIT);
  const hasMoreExamples = (examples?.length ?? 0) > EXAMPLE_PREVIEW_LIMIT;

  const statusItems = [
    { value: "all" as const, label: `All` },
    { value: "active" as const, label: `Active` },
    { value: "archived" as const, label: `Archived` },
  ];

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Agents" }]} />

      <PageMasthead
        icon={<Boxes />}
        title="Agents"
        badges={
          <Badge variant="outline" className="font-mono">
            {counts.all}
          </Badge>
        }
        description="Reusable agent definitions — instructions, capabilities, and model."
        meta={
          <>
            <span>{counts.active} active</span>
            <span>{counts.archived} archived</span>
          </>
        }
        actions={
          <>
            <input
              type="file"
              ref={fileInputRef}
              onChange={handleFileChange}
              accept=".md,.yaml,.yml,.json"
              className="hidden"
              aria-label="Import agent file"
            />
            <Button variant="outline" onClick={handleImportClick} disabled={importAgent.isPending}>
              <Upload className="size-4" />
              {importAgent.isPending ? "Importing..." : "Import"}
            </Button>
            <NewAgentLink>
              <Button variant="accent">
                <Plus className="size-4" />
                New agent
              </Button>
            </NewAgentLink>
          </>
        }
      />

      {importError && (
        <div className="border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">
          {importError}
        </div>
      )}

      <PageControlStrip className="flex flex-wrap items-center gap-3">
        <SearchInput
          placeholder="Search agents…"
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

      <PageColumns>
        <PageMain>
          <QueryStateWrapper
            isLoading={isLoading}
            error={error}
            data={filteredAgents}
            errorMessagePrefix="Failed to load agents"
            emptyState={
              <EmptyState
                icon={<Boxes />}
                title={
                  search || statusTab !== "active"
                    ? "No agents match your filters."
                    : "No agents yet"
                }
                action={
                  !search &&
                  statusTab === "active" && (
                    <NewAgentLink>
                      <Button variant="accent">
                        <Plus className="size-4" />
                        Create your first agent
                      </Button>
                    </NewAgentLink>
                  )
                }
              />
            }
          >
            {(items) => (
              <div className={cn("grid gap-4", view === "grid" ? "md:grid-cols-2" : "grid-cols-1")}>
                {items.map((agent, index) => (
                  <AgentCard
                    key={agent.id ?? `agent-${index}`}
                    agent={agent}
                    allCapabilities={allCapabilities}
                    showEditButton
                  />
                ))}
              </div>
            )}
          </QueryStateWrapper>
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

          {capabilityFacets.length > 0 && (
            <RailSection label="Capability">
              <div className="flex flex-col gap-1.5 text-[13px] text-muted-foreground">
                {capabilityFacets.map((facet) => {
                  const Icon = facet.icon;
                  return (
                    <div key={facet.ref} className="flex items-center justify-between">
                      <span className="inline-flex items-center gap-1.5">
                        <Icon className="size-3.5" />
                        {facet.name}
                      </span>
                      <span>{facet.count}</span>
                    </div>
                  );
                })}
              </div>
            </RailSection>
          )}

          {tagFacets.length > 0 && (
            <RailSection label="Tags">
              <div className="flex flex-wrap gap-1.5">
                {tagFacets.map((tag) => (
                  <span key={tag} className="border bg-muted px-2 py-0.5 text-xs">
                    {tag}
                  </span>
                ))}
              </div>
            </RailSection>
          )}
        </PageRail>
      </PageColumns>

      <PageFooter>
        <span>
          Showing {filteredAgents.length} of {counts.all} {pluralize(counts.all, "agent")}
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

      {/* Example agents — a secondary discovery section below the primary frame. */}
      <section className="mt-4">
        <div className="mb-4 flex items-center justify-between">
          <h2 className="text-lg font-semibold tracking-tight">Example agents</h2>
          {hasMoreExamples && (
            <Link
              href="/agents/examples"
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
                <ExampleCard
                  key={example.name ?? `example-${index}`}
                  example={example}
                  allCapabilities={allCapabilities}
                  onImport={handleImport}
                  adopting={importingName === example.name}
                />
              ))}
            </div>
          )}
        </QueryStateWrapper>
      </section>
    </PageContainer>
  );
}
