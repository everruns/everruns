"use client";

import { useState } from "react";
import { useParams } from "next/navigation";
import Link from "next/link";
import { useAgent, useAgentVersions, useCreateAgentVersion } from "@/hooks/use-agents";
import { Header } from "@/components/layout/header";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { ArrowLeft, Bot, Plus, Settings, Loader2, Clock } from "lucide-react";
import type { AgentVersion } from "@/lib/api/types";

function VersionCard({ version }: { version: AgentVersion }) {
  const [showDefinition, setShowDefinition] = useState(false);

  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
        <div className="flex items-center gap-3">
          <Badge variant="outline">v{version.version}</Badge>
          <span className="text-sm text-muted-foreground">
            <Clock className="inline h-3 w-3 mr-1" />
            {new Date(version.created_at).toLocaleString()}
          </span>
        </div>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => setShowDefinition(!showDefinition)}
        >
          {showDefinition ? "Hide" : "Show"} Definition
        </Button>
      </CardHeader>
      {showDefinition && (
        <CardContent>
          <pre className="bg-muted p-4 rounded-lg text-sm overflow-auto max-h-64">
            {JSON.stringify(version.definition, null, 2)}
          </pre>
        </CardContent>
      )}
    </Card>
  );
}

function CreateVersionDialog({ agentId }: { agentId: string }) {
  const [open, setOpen] = useState(false);
  const [systemPrompt, setSystemPrompt] = useState("");
  const createVersion = useCreateAgentVersion(agentId);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();

    try {
      await createVersion.mutateAsync({
        definition: {
          system_prompt: systemPrompt,
          tools: [],
        },
      });
      setOpen(false);
      setSystemPrompt("");
    } catch (error) {
      console.error("Failed to create version:", error);
    }
  };

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button>
          <Plus className="h-4 w-4 mr-2" />
          New Version
        </Button>
      </DialogTrigger>
      <DialogContent className="sm:max-w-lg">
        <form onSubmit={handleSubmit}>
          <DialogHeader>
            <DialogTitle>Create New Version</DialogTitle>
            <DialogDescription>
              Create a new version of this agent with an updated definition.
            </DialogDescription>
          </DialogHeader>
          <div className="py-4 space-y-4">
            <div className="space-y-2">
              <Label htmlFor="system-prompt">System Prompt</Label>
              <Textarea
                id="system-prompt"
                placeholder="You are a helpful assistant..."
                value={systemPrompt}
                onChange={(e) => setSystemPrompt(e.target.value)}
                rows={6}
                required
              />
            </div>
          </div>
          {createVersion.error && (
            <div className="bg-destructive/10 text-destructive p-3 rounded-lg text-sm mb-4">
              Failed to create version: {createVersion.error.message}
            </div>
          )}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => setOpen(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={createVersion.isPending}>
              {createVersion.isPending && <Loader2 className="h-4 w-4 mr-2 animate-spin" />}
              Create Version
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export default function AgentDetailPage() {
  const params = useParams();
  const agentId = params.agentId as string;

  const { data: agent, isLoading: agentLoading, error: agentError } = useAgent(agentId);
  const { data: versions = [], isLoading: versionsLoading } = useAgentVersions(agentId);

  if (agentLoading) {
    return (
      <>
        <Header title="Agent Details" />
        <div className="p-6 space-y-6">
          <Skeleton className="h-48" />
          <Skeleton className="h-64" />
        </div>
      </>
    );
  }

  if (agentError || !agent) {
    return (
      <>
        <Header
          title="Agent Not Found"
          action={
            <Link href="/agents">
              <Button variant="ghost">
                <ArrowLeft className="h-4 w-4 mr-2" />
                Back to Agents
              </Button>
            </Link>
          }
        />
        <div className="p-6">
          <div className="bg-destructive/10 text-destructive p-4 rounded-lg">
            {agentError?.message || "Agent not found"}
          </div>
        </div>
      </>
    );
  }

  return (
    <>
      <Header
        title={agent.name}
        action={
          <div className="flex gap-2">
            <Link href="/agents">
              <Button variant="ghost">
                <ArrowLeft className="h-4 w-4 mr-2" />
                Back
              </Button>
            </Link>
            <Link href={`/agents/${agentId}/edit`}>
              <Button variant="outline">
                <Settings className="h-4 w-4 mr-2" />
                Edit
              </Button>
            </Link>
          </div>
        }
      />
      <div className="p-6 space-y-6">
        {/* Agent Info Card */}
        <Card>
          <CardHeader className="flex flex-row items-start justify-between">
            <div className="flex items-center gap-4">
              <div className="p-3 bg-primary/10 rounded-lg">
                <Bot className="h-8 w-8 text-primary" />
              </div>
              <div>
                <CardTitle className="text-2xl">{agent.name}</CardTitle>
                <CardDescription className="mt-1">
                  {agent.description || "No description"}
                </CardDescription>
              </div>
            </div>
            <Badge
              variant="outline"
              className={
                agent.status === "active"
                  ? "bg-green-100 text-green-800"
                  : "bg-gray-100 text-gray-800"
              }
            >
              {agent.status}
            </Badge>
          </CardHeader>
          <CardContent>
            <div className="grid grid-cols-2 md:grid-cols-4 gap-4 text-sm">
              <div>
                <p className="text-muted-foreground">Model</p>
                <p className="font-medium">{agent.default_model_id}</p>
              </div>
              <div>
                <p className="text-muted-foreground">Versions</p>
                <p className="font-medium">{versions.length}</p>
              </div>
              <div>
                <p className="text-muted-foreground">Created</p>
                <p className="font-medium">
                  {new Date(agent.created_at).toLocaleDateString()}
                </p>
              </div>
              <div>
                <p className="text-muted-foreground">Updated</p>
                <p className="font-medium">
                  {new Date(agent.updated_at).toLocaleDateString()}
                </p>
              </div>
            </div>
          </CardContent>
        </Card>

        {/* Versions Section */}
        <div className="space-y-4">
          <div className="flex items-center justify-between">
            <h2 className="text-xl font-semibold">Versions</h2>
            <CreateVersionDialog agentId={agentId} />
          </div>

          {versionsLoading ? (
            <div className="space-y-3">
              {[...Array(3)].map((_, i) => (
                <Skeleton key={i} className="h-16" />
              ))}
            </div>
          ) : versions.length === 0 ? (
            <Card>
              <CardContent className="py-8 text-center">
                <p className="text-muted-foreground mb-4">
                  No versions yet. Create a version to define the agent's behavior.
                </p>
                <CreateVersionDialog agentId={agentId} />
              </CardContent>
            </Card>
          ) : (
            <div className="space-y-3">
              {versions
                .slice()
                .sort((a, b) => b.version - a.version)
                .map((version) => (
                  <VersionCard key={version.version} version={version} />
                ))}
            </div>
          )}
        </div>
      </div>
    </>
  );
}
