"use client";

import { useRef, useState, useCallback } from "react";
import {
  useAgents,
  useAgentTemplates,
  useInstallAgentTemplate,
  useCapabilities,
  useImportAgent,
} from "@/hooks";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Plus, Upload } from "lucide-react";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { AgentCard, TemplateCard } from "@/components/agents";
import { ArchiveFilter } from "@/components/archive-filter";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";

export default function AgentsPage() {
  const router = useRouter();
  const [tab, setTab] = useState("agents");
  const [showArchived, setShowArchived] = useState(false);
  const { data: agents, isLoading, error } = useAgents({ includeArchived: showArchived });
  const { data: allCapabilities } = useCapabilities();
  const {
    data: templates,
    isLoading: templatesLoading,
    error: templatesError,
  } = useAgentTemplates();
  const importAgent = useImportAgent();
  const installTemplate = useInstallAgentTemplate();
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [importError, setImportError] = useState<string | null>(null);
  const [installingSlug, setInstallingSlug] = useState<string | null>(null);

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

  const handleInstall = useCallback(
    async (slug: string) => {
      setInstallingSlug(slug);
      try {
        const agent = await installTemplate.mutateAsync(slug);
        router.push(`/agents/${agent.id}`);
      } catch (err) {
        console.error("Failed to install template:", err);
      } finally {
        setInstallingSlug(null);
      }
    },
    [installTemplate, router],
  );

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
        </div>
      </div>

      {importError && (
        <div className="mb-4 p-4 bg-red-50 border border-red-200 rounded-md text-red-600 text-sm">
          {importError}
        </div>
      )}

      <Tabs value={tab} onValueChange={setTab}>
        <TabsList>
          <TabsTrigger value="agents">My Agents</TabsTrigger>
          <TabsTrigger value="templates">Templates</TabsTrigger>
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
                  <Button variant="outline" onClick={() => setTab("templates")}>
                    Browse templates
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

        <TabsContent value="templates">
          <QueryStateWrapper
            isLoading={templatesLoading}
            error={templatesError}
            data={templates}
            errorMessagePrefix="Failed to load templates"
            emptyState={
              <div className="text-center py-12">
                <p className="text-muted-foreground">No templates available</p>
              </div>
            }
          >
            {(items) => (
              <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
                {items.map((template) => (
                  <TemplateCard
                    key={template.slug}
                    template={template}
                    allCapabilities={allCapabilities}
                    onInstall={handleInstall}
                    installing={installingSlug === template.slug}
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
