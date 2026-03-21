"use client";

import { useRef, useState, useCallback, useMemo } from "react";
import {
  useAgents,
  useAgentExamples,
  useAdoptAgentExample,
  useCapabilities,
  useImportAgent,
} from "@/hooks";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Plus, Upload, Search } from "lucide-react";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { AgentCard, ExampleCard } from "@/components/agents";
import { ArchiveFilter } from "@/components/archive-filter";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";

export default function AgentsPage() {
  const router = useRouter();
  const [tab, setTab] = useState("agents");
  const [showArchived, setShowArchived] = useState(false);
  const [exampleSearch, setExampleSearch] = useState("");
  const { data: agents, isLoading, error } = useAgents({ includeArchived: showArchived });
  const { data: allCapabilities } = useCapabilities();
  const { data: examples, isLoading: examplesLoading, error: examplesError } = useAgentExamples();
  const importAgent = useImportAgent();
  const adoptExample = useAdoptAgentExample();
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [importError, setImportError] = useState<string | null>(null);
  const [adoptingSlug, setAdoptingSlug] = useState<string | null>(null);

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

      // Reset file input
      if (fileInputRef.current) {
        fileInputRef.current.value = "";
      }
    },
    [importAgent, router],
  );

  const handleUse = useCallback(
    async (slug: string) => {
      setAdoptingSlug(slug);
      try {
        const agent = await adoptExample.mutateAsync(slug);
        router.push(`/agents/${agent.id}`);
      } catch (err) {
        console.error("Failed to use example:", err);
      } finally {
        setAdoptingSlug(null);
      }
    },
    [adoptExample, router],
  );

  const filteredExamples = useMemo(() => {
    if (!examples || !exampleSearch.trim()) return examples;
    const query = exampleSearch.toLowerCase();
    return examples.filter(
      (ex) =>
        ex.name.toLowerCase().includes(query) ||
        ex.description.toLowerCase().includes(query) ||
        ex.tags.some((tag) => tag.toLowerCase().includes(query)),
    );
  }, [examples, exampleSearch]);

  return (
    <div className="container mx-auto p-6">
      <div className="flex items-center justify-between mb-6">
        <h1 className="text-2xl font-bold">Agents</h1>
        <div className="flex items-center gap-2">
          {tab === "agents" && (
            <>
              <ArchiveFilter showArchived={showArchived} onShowArchivedChange={setShowArchived} />
              <input
                type="file"
                ref={fileInputRef}
                onChange={handleFileChange}
                accept=".md,.yaml,.yml,.json"
                className="hidden"
              />
              <Button
                variant="outline"
                onClick={handleImportClick}
                disabled={importAgent.isPending}
              >
                <Upload className="w-4 h-4 mr-2" />
                {importAgent.isPending ? "Importing..." : "Import"}
              </Button>
              <Link href="/agents/new">
                <Button variant="accent">
                  <Plus className="w-4 h-4 mr-2" />
                  New Agent
                </Button>
              </Link>
            </>
          )}
          {tab === "examples" && (
            <div className="relative">
              <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
              <Input
                placeholder="Search examples..."
                value={exampleSearch}
                onChange={(e) => setExampleSearch(e.target.value)}
                className="pl-8 w-64"
              />
            </div>
          )}
        </div>
      </div>

      {importError && (
        <div className="mb-4 p-4 bg-red-50 border border-red-200 rounded-md text-red-600 text-sm">
          {importError}
        </div>
      )}

      <Tabs value={tab} onValueChange={setTab}>
        <TabsList>
          <TabsTrigger value="agents">Agents</TabsTrigger>
          <TabsTrigger value="examples">Examples</TabsTrigger>
        </TabsList>

        <TabsContent value="agents">
          <QueryStateWrapper
            isLoading={isLoading}
            error={error}
            data={agents}
            errorMessagePrefix="Failed to load agents"
            emptyState={
              <div className="text-center py-12">
                <p className="text-muted-foreground mb-4">No agents yet</p>
                <div className="flex items-center justify-center gap-2">
                  <Link href="/agents/new">
                    <Button>
                      <Plus className="w-4 h-4 mr-2" />
                      Create your first agent
                    </Button>
                  </Link>
                  <Button variant="outline" onClick={() => setTab("examples")}>
                    Browse examples
                  </Button>
                </div>
              </div>
            }
          >
            {(items) => (
              <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
                {items.map((agent) => (
                  <AgentCard
                    key={agent.id}
                    agent={agent}
                    allCapabilities={allCapabilities}
                    showEditButton
                  />
                ))}
              </div>
            )}
          </QueryStateWrapper>
        </TabsContent>

        <TabsContent value="examples">
          <QueryStateWrapper
            isLoading={examplesLoading}
            error={examplesError}
            data={filteredExamples}
            errorMessagePrefix="Failed to load examples"
            emptyState={
              <div className="text-center py-12">
                <p className="text-muted-foreground">
                  {exampleSearch.trim() ? "No examples match your search" : "No examples available"}
                </p>
              </div>
            }
          >
            {(items) => (
              <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
                {items.map((example) => (
                  <ExampleCard
                    key={example.slug}
                    example={example}
                    allCapabilities={allCapabilities}
                    onUse={handleUse}
                    adopting={adoptingSlug === example.slug}
                  />
                ))}
              </div>
            )}
          </QueryStateWrapper>
        </TabsContent>
      </Tabs>
    </div>
  );
}
