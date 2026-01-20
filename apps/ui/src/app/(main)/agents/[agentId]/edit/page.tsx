"use client";

import { use, useState, useMemo, useCallback } from "react";
import { useRouter } from "next/navigation";
import Link from "next/link";
import {
  useAgent,
  useUpdateAgent,
  useDeleteAgent,
  useCapabilities,
} from "@/hooks";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { PromptEditor } from "@/components/ui/prompt-editor";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import { CapabilitySelector } from "@/components/agents/capability-selector";
import { AgentPreview } from "@/components/agents/agent-preview";
import { ModelPicker } from "@/components/models/model-picker";
import { ArrowLeft, Save, Trash2, Eye, Edit2 } from "lucide-react";
import type { AgentCapabilityConfig } from "@/lib/api/types";

interface FormData {
  name: string;
  description: string;
  system_prompt: string;
  tags: string;
  default_model_id: string;
}

export default function EditAgentPage({
  params,
}: {
  params: Promise<{ agentId: string }>;
}) {
  const { agentId } = use(params);
  const router = useRouter();

  // Agent data
  const { data: agent, isLoading: agentLoading } = useAgent(agentId);
  const updateAgent = useUpdateAgent();
  const deleteAgent = useDeleteAgent();

  // Capabilities data
  const { data: allCapabilities, isLoading: capabilitiesLoading } =
    useCapabilities();

  // Tab state
  const [activeTab, setActiveTab] = useState<string>("edit");

  // Form state - track user changes separately from initial values
  const [formChanges, setFormChanges] = useState<Partial<FormData>>({});

  // Compute initial values from agent data
  const initialFormData = useMemo((): FormData => {
    if (!agent) {
      return { name: "", description: "", system_prompt: "", tags: "", default_model_id: "" };
    }
    return {
      name: agent.name,
      description: agent.description || "",
      system_prompt: agent.system_prompt,
      tags: agent.tags.join(", "),
      default_model_id: agent.default_model_id || "",
    };
  }, [agent]);

  // Merge initial values with user changes
  const formData = useMemo(
    () => ({ ...initialFormData, ...formChanges }),
    [initialFormData, formChanges]
  );

  const handleFormChange = useCallback(
    (field: keyof FormData, value: string) => {
      setFormChanges((prev) => ({ ...prev, [field]: value }));
    },
    []
  );

  // Capabilities state - now included directly in agent response
  // Use full AgentCapabilityConfig objects to preserve per-agent config
  const initialCapabilities = useMemo(() => {
    return agent?.capabilities ?? [];
  }, [agent?.capabilities]);

  const [localCapabilities, setLocalCapabilities] = useState<
    AgentCapabilityConfig[] | null
  >(null);
  const selectedCapabilities = localCapabilities ?? initialCapabilities;

  // Capabilities change handler
  const handleCapabilitiesChange = useCallback((newCapabilities: AgentCapabilityConfig[]) => {
    setLocalCapabilities(newCapabilities);
  }, []);

  // Submit handler
  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();

    try {
      // Parse tags
      const tags = formData.tags
        .split(",")
        .map((t) => t.trim())
        .filter((t) => t.length > 0);

      // Get capabilities to save (already full AgentCapabilityConfig format)
      const capabilitiesToSave = localCapabilities ?? initialCapabilities;
      const capabilitiesChanged =
        JSON.stringify(capabilitiesToSave) !== JSON.stringify(initialCapabilities);

      // Update agent (capabilities are now part of the agent resource)
      await updateAgent.mutateAsync({
        agentId,
        request: {
          name: formData.name,
          description: formData.description || undefined,
          system_prompt: formData.system_prompt,
          tags,
          default_model_id: formData.default_model_id || undefined,
          // Only include capabilities if they changed
          ...(capabilitiesChanged && { capabilities: capabilitiesToSave }),
        },
      });

      router.push(`/agents/${agentId}`);
    } catch (error) {
      console.error("Failed to update agent:", error);
    }
  };

  // Delete handler
  const handleDelete = async () => {
    if (!confirm("Are you sure you want to delete this agent? This action cannot be undone.")) {
      return;
    }

    try {
      await deleteAgent.mutateAsync(agentId);
      router.push("/agents");
    } catch (error) {
      console.error("Failed to delete agent:", error);
    }
  };

  const isLoading = agentLoading || capabilitiesLoading;
  const isSaving = updateAgent.isPending;

  if (isLoading) {
    return (
      <div className="container mx-auto p-6">
        <Skeleton className="h-8 w-1/4 mb-6" />
        <div className="grid gap-6 lg:grid-cols-3">
          <div className="lg:col-span-2">
            <Skeleton className="h-[500px] w-full" />
          </div>
          <div>
            <Skeleton className="h-[300px] w-full" />
          </div>
        </div>
      </div>
    );
  }

  if (!agent) {
    return (
      <div className="container mx-auto p-6">
        <div className="text-red-500">Agent not found</div>
        <Link href="/agents" className="text-blue-500 hover:underline">
          Back to agents
        </Link>
      </div>
    );
  }

  return (
    <div className="container mx-auto p-6">
      <Link
        href={`/agents/${agentId}`}
        className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-6"
      >
        <ArrowLeft className="w-4 h-4 mr-2" />
        Back to Agent
      </Link>

      <div className="flex items-center justify-between mb-6">
        <h1 className="text-2xl font-bold">Edit Agent</h1>
        <Tabs value={activeTab} onValueChange={setActiveTab}>
          <TabsList>
            <TabsTrigger value="edit">
              <Edit2 className="w-4 h-4 mr-2" />
              Edit
            </TabsTrigger>
            <TabsTrigger value="preview">
              <Eye className="w-4 h-4 mr-2" />
              Preview
            </TabsTrigger>
          </TabsList>
        </Tabs>
      </div>

      <Tabs value={activeTab} onValueChange={setActiveTab}>
        {/* Edit Tab Content */}
        <TabsContent value="edit">
          <form onSubmit={handleSubmit}>
            <div className="grid gap-6 lg:grid-cols-3">
              {/* Main form */}
              <div className="lg:col-span-2 space-y-6">
                <Card>
                  <CardHeader>
                    <CardTitle>Agent Details</CardTitle>
                  </CardHeader>
                  <CardContent className="space-y-6">
                    <div className="space-y-2">
                      <Label htmlFor="name">Name</Label>
                      <Input
                        id="name"
                        placeholder="My Agent"
                        value={formData.name}
                        onChange={(e) => handleFormChange("name", e.target.value)}
                        required
                      />
                    </div>

                    <div className="space-y-2">
                      <Label htmlFor="description">Description</Label>
                      <Textarea
                        id="description"
                        placeholder="Describe what this agent does..."
                        value={formData.description}
                        onChange={(e) =>
                          handleFormChange("description", e.target.value)
                        }
                        rows={2}
                      />
                    </div>

                    <div className="space-y-2">
                      <Label htmlFor="tags">Tags</Label>
                      <Input
                        id="tags"
                        placeholder="tag1, tag2, tag3"
                        value={formData.tags}
                        onChange={(e) => handleFormChange("tags", e.target.value)}
                      />
                      <p className="text-xs text-muted-foreground">
                        Comma-separated list of tags
                      </p>
                    </div>

                    <div className="space-y-2">
                      <Label htmlFor="model">Model (optional)</Label>
                      <ModelPicker
                        value={formData.default_model_id || ""}
                        onChange={(value) => handleFormChange("default_model_id", value)}
                        placeholder="Use default model"
                      />
                      <p className="text-xs text-muted-foreground">
                        Select a specific model or leave empty to use the default
                      </p>
                    </div>

                    <div className="space-y-2">
                      <Label htmlFor="system_prompt">System Prompt</Label>
                      <PromptEditor
                        id="system_prompt"
                        placeholder="You are a helpful assistant..."
                        value={formData.system_prompt}
                        onChange={(value) =>
                          handleFormChange("system_prompt", value)
                        }
                        required
                      />
                      <p className="text-xs text-muted-foreground">
                        Instructions for the AI model (supports Markdown)
                      </p>
                    </div>
                  </CardContent>
                </Card>

                {/* Danger Zone */}
                <Card className="border-destructive/50">
                  <CardHeader>
                    <CardTitle className="text-destructive">Danger Zone</CardTitle>
                    <CardDescription>
                      Irreversible actions that affect this agent
                    </CardDescription>
                  </CardHeader>
                  <CardContent>
                    <div className="flex items-center justify-between">
                      <div>
                        <p className="font-medium">Delete this agent</p>
                        <p className="text-sm text-muted-foreground">
                          Once deleted, this agent and all its sessions will be permanently removed.
                        </p>
                      </div>
                      <Button
                        type="button"
                        variant="destructive"
                        onClick={handleDelete}
                        disabled={deleteAgent.isPending}
                      >
                        <Trash2 className="w-4 h-4 mr-2" />
                        {deleteAgent.isPending ? "Deleting..." : "Delete Agent"}
                      </Button>
                    </div>
                  </CardContent>
                </Card>
              </div>

              {/* Capabilities sidebar */}
              <div className="space-y-6">
                <Card>
                  <CardHeader>
                    <CardTitle>Capabilities</CardTitle>
                  </CardHeader>
                  <CardContent>
                    <CapabilitySelector
                      capabilities={allCapabilities || []}
                      selected={selectedCapabilities}
                      onChange={handleCapabilitiesChange}
                      disabled={isSaving}
                    />
                  </CardContent>
                </Card>

                {/* Save button */}
                <div className="flex gap-4">
                  <Button type="submit" disabled={isSaving} className="flex-1">
                    <Save className="w-4 h-4 mr-2" />
                    {isSaving ? "Saving..." : "Save Changes"}
                  </Button>
                  <Button
                    type="button"
                    variant="outline"
                    onClick={() => router.back()}
                  >
                    Cancel
                  </Button>
                </div>

                {updateAgent.error && (
                  <p className="text-sm text-destructive">
                    Error: {updateAgent.error.message}
                  </p>
                )}
              </div>
            </div>
          </form>
        </TabsContent>

        {/* Preview Tab Content */}
        <TabsContent value="preview">
          <div className="grid gap-6 lg:grid-cols-3">
            <div className="lg:col-span-2">
              <AgentPreview
                systemPrompt={formData.system_prompt}
                capabilities={selectedCapabilities}
              />
            </div>

            {/* Summary sidebar */}
            <div className="space-y-6">
              <Card>
                <CardHeader>
                  <CardTitle>Agent Summary</CardTitle>
                  <CardDescription>
                    Current configuration
                  </CardDescription>
                </CardHeader>
                <CardContent className="space-y-4">
                  <div>
                    <p className="text-sm font-medium">Name</p>
                    <p className="text-sm text-muted-foreground">{formData.name || "(not set)"}</p>
                  </div>
                  <div>
                    <p className="text-sm font-medium">Description</p>
                    <p className="text-sm text-muted-foreground">{formData.description || "(not set)"}</p>
                  </div>
                  <div>
                    <p className="text-sm font-medium">Capabilities</p>
                    <p className="text-sm text-muted-foreground">
                      {selectedCapabilities.length} capabilit{selectedCapabilities.length !== 1 ? "ies" : "y"} enabled
                    </p>
                  </div>
                </CardContent>
              </Card>

              <Card className="border-dashed">
                <CardContent className="pt-6">
                  <p className="text-sm text-muted-foreground text-center">
                    This preview shows what the final agent will look like after applying all capabilities.
                    Switch to the Edit tab to make changes.
                  </p>
                </CardContent>
              </Card>
            </div>
          </div>
        </TabsContent>
      </Tabs>
    </div>
  );
}
