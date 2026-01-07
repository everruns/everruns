"use client";

import { useRef, useState, useCallback } from "react";
import { useAgents, useCapabilities, useImportAgent } from "@/hooks";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Plus, Upload } from "lucide-react";
import { AgentCard } from "@/components/agents";

export default function AgentsPage() {
  const router = useRouter();
  const { data: agents, isLoading, error } = useAgents();
  const { data: allCapabilities } = useCapabilities();
  const importAgent = useImportAgent();
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [importError, setImportError] = useState<string | null>(null);

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
        setImportError(
          err instanceof Error ? err.message : "Failed to import agent"
        );
      }

      // Reset file input
      if (fileInputRef.current) {
        fileInputRef.current.value = "";
      }
    },
    [importAgent, router]
  );

  if (error) {
    return (
      <div className="container mx-auto p-6">
        <div className="text-red-500">
          Error loading agents: {error.message}
        </div>
      </div>
    );
  }

  return (
    <div className="container mx-auto p-6">
      <div className="flex items-center justify-between mb-6">
        <h1 className="text-2xl font-bold">Agents</h1>
        <div className="flex gap-2">
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
            <Button>
              <Plus className="w-4 h-4 mr-2" />
              New Agent
            </Button>
          </Link>
        </div>
      </div>

      {importError && (
        <div className="mb-4 p-4 bg-red-50 border border-red-200 rounded-md text-red-600 text-sm">
          {importError}
        </div>
      )}

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
      ) : agents?.length === 0 ? (
        <div className="text-center py-12">
          <p className="text-muted-foreground mb-4">No agents yet</p>
          <Link href="/agents/new">
            <Button>
              <Plus className="w-4 h-4 mr-2" />
              Create your first agent
            </Button>
          </Link>
        </div>
      ) : (
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {agents?.map((agent) => (
            <AgentCard
              key={agent.id}
              agent={agent}
              allCapabilities={allCapabilities}
              showEditButton
            />
          ))}
        </div>
      )}
    </div>
  );
}
