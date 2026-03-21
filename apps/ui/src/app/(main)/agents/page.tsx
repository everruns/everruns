"use client";

import { useRef, useState, useCallback } from "react";
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
import { Plus, Upload, ArrowRight } from "lucide-react";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { AgentCard, ExampleCard } from "@/components/agents";

const PREVIEW_LIMIT = 6;

export default function AgentsPage() {
  const router = useRouter();
  const { data: agents, isLoading, error } = useAgents({ includeArchived: false });
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

  const previewAgents = agents?.slice(0, PREVIEW_LIMIT);
  const previewExamples = examples?.slice(0, PREVIEW_LIMIT);
  const hasMoreAgents = (agents?.length ?? 0) > PREVIEW_LIMIT;
  const hasMoreExamples = (examples?.length ?? 0) > PREVIEW_LIMIT;

  return (
    <div className="container mx-auto p-6 space-y-10">
      {/* Header */}
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">Agents</h1>
        <div className="flex items-center gap-2">
          <input
            type="file"
            ref={fileInputRef}
            onChange={handleFileChange}
            accept=".md,.yaml,.yml,.json"
            className="hidden"
          />
          <Button variant="outline" onClick={handleImportClick} disabled={importAgent.isPending}>
            <Upload className="w-4 h-4 mr-2" />
            {importAgent.isPending ? "Importing..." : "Import"}
          </Button>
          <Link href="/agents/new">
            <Button variant="accent">
              <Plus className="w-4 h-4 mr-2" />
              New Agent
            </Button>
          </Link>
        </div>
      </div>

      {importError && (
        <div className="p-4 bg-red-50 border border-red-200 rounded-md text-red-600 text-sm">
          {importError}
        </div>
      )}

      {/* My Agents Section */}
      <section>
        <QueryStateWrapper
          isLoading={isLoading}
          error={error}
          data={previewAgents}
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
        {hasMoreAgents && (
          <div className="flex justify-end mt-4">
            <Link href="/agents/all">
              <Button variant="outline">
                All agents
                <ArrowRight className="w-4 h-4 ml-2" />
              </Button>
            </Link>
          </div>
        )}
      </section>

      {/* Divider */}
      <hr className="border-border" />

      {/* Example Agents Section */}
      <section>
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-xl font-semibold">Example agents</h2>
        </div>
        <QueryStateWrapper
          isLoading={examplesLoading}
          error={examplesError}
          data={previewExamples}
          errorMessagePrefix="Failed to load examples"
          emptyState={
            <div className="text-center py-12">
              <p className="text-muted-foreground">No examples available</p>
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
        {hasMoreExamples && (
          <div className="flex justify-end mt-4">
            <Link href="/agents/examples">
              <Button variant="outline">
                All examples
                <ArrowRight className="w-4 h-4 ml-2" />
              </Button>
            </Link>
          </div>
        )}
      </section>
    </div>
  );
}
