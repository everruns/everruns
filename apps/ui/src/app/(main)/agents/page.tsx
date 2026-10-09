"use client";

import { AgentIcon } from "@/components/icons/facet-icons";
import { Suspense, useRef, useState, useCallback, useMemo } from "react";
import {
  useAgents,
  useAgentExamples,
  useImportAgentExample,
  useCapabilities,
  useImportAgent,
  usePageTitle,
} from "@/hooks";
import { useRouter } from "next/navigation";
import { importedExampleLanding } from "@/lib/agent-template-setup";
import Link from "next/link";
import { NewAgentLink } from "@/components/agents/new-agent-link";
import { Button } from "@/components/ui/button";
import { SearchInput } from "@/components/ui/search-input";
import { Plus, Upload, ArrowRight, LayoutGrid, List as ListIcon } from "lucide-react";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { AgentCard, ExampleCard } from "@/components/agents";
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
import { cn } from "@/lib/utils";
import { AgentImportDialog } from "@/components/agents/agent-import-dialog";
import { AgentsHome } from "@/components/agents/home/agents-home";
import { useFeatureFlagsState } from "@/providers/feature-flags-provider";
import type { Agent } from "@/lib/api/types";

const EXAMPLE_PREVIEW_LIMIT = 3;
type StatusTab = "all" | "active" | "archived";

export default function AgentsPage() {
  const { flags, isLoading } = useFeatureFlagsState();
  // Wait for the org's flags so an opted-in org never flashes the old page.
  if (isLoading) return null;
  if (flags.agents_home) {
    return (
      <Suspense fallback={null}>
        <AgentsHome />
      </Suspense>
    );
  }
  return <AgentsRegistry />;
}

/** The agent registry page, shown while the `agents_home` flag is off. */
function AgentsRegistry() {
  usePageTitle("Agents");
  const { locale } = useLocale();
  const router = useRouter();
  // Tabs count the same complete set they filter.
  const { data: agents, isLoading, error } = useAgents({ includeArchived: true });
  const { data: allCapabilities } = useCapabilities({ includeRetired: true });
  const { data: examples, isLoading: examplesLoading, error: examplesError } = useAgentExamples();
  const importAgent = useImportAgent();
  const importExample = useImportAgentExample();
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [importFile, setImportFile] = useState<File | null>(null);
  const [importingName, setImportingName] = useState<string | null>(null);

  const [search, setSearch] = useState("");
  const [statusTab, setStatusTab] = useState<StatusTab>("active");
  const [view, setView] = useState<"grid" | "list">("grid");

  const handleImportClick = useCallback(() => {
    fileInputRef.current?.click();
  }, []);

  const handleFileChange = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (file) setImportFile(file);
    e.target.value = "";
  }, []);

  const handleImport = useCallback(
    async (name: string) => {
      setImportingName(name);
      try {
        const agent = await importExample.mutateAsync(name);
        router.push(importedExampleLanding(examples, name, agent.id));
      } catch (err) {
        console.error("Failed to import example:", err);
      } finally {
        setImportingName(null);
      }
    },
    [importExample, router, examples],
  );

  const counts = useMemo(() => {
    const list = agents ?? [];
    const archived = list.filter((a) => isArchivedStatus(a.status)).length;
    return { all: list.length, active: list.length - archived, archived };
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
        icon={<AgentIcon />}
        title="Agents"
        description="Reusable agent definitions — instructions, capabilities, and model."
        actions={
          <>
            <input
              type="file"
              ref={fileInputRef}
              onChange={handleFileChange}
              accept=".md,.toml,.yaml,.yml,.json,.zip"
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

      {importFile && (
        <AgentImportDialog
          file={importFile}
          agents={agents ?? []}
          capabilities={allCapabilities ?? []}
          onClose={() => setImportFile(null)}
          onImported={(agent) => router.push(`/agents/${agent.id}`)}
        />
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
          data={filteredAgents}
          errorMessagePrefix="Failed to load agents"
          emptyState={
            <EmptyState
              icon={<AgentIcon />}
              title={
                search || statusTab !== "active" ? "No agents match your filters." : "No agents yet"
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

      {/* Example agents — a secondary discovery section below the primary frame. */}
      <section>
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
