"use client";

import { use, useState, useMemo, useCallback } from "react";
import { useRouter } from "next/navigation";
import Link from "next/link";
import {
  useAgent,
  useUpdateAgent,
  useDeleteAgent,
  useCapabilities,
  useAgentCapabilities,
  useSetAgentCapabilities,
} from "@/hooks";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { PromptEditor } from "@/components/ui/prompt-editor";
import { Checkbox } from "@/components/ui/checkbox";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import {
  ArrowLeft,
  Save,
  Trash2,
  CircleOff,
  Clock,
  Search,
  Box,
  Folder,
  ChevronUp,
  ChevronDown,
  LucideIcon,
} from "lucide-react";
import type { Capability, CapabilityId } from "@/lib/api/types";

const iconMap: Record<string, LucideIcon> = {
  "circle-off": CircleOff,
  clock: Clock,
  search: Search,
  box: Box,
  folder: Folder,
};

interface FormData {
  name: string;
  description: string;
  system_prompt: string;
  tags: string;
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
  const { data: agentCapabilities, isLoading: agentCapabilitiesLoading } =
    useAgentCapabilities(agentId);
  const setCapabilities = useSetAgentCapabilities();

  // Form state - track user changes separately from initial values
  const [formChanges, setFormChanges] = useState<Partial<FormData>>({});

  // Compute initial values from agent data
  const initialFormData = useMemo((): FormData => {
    if (!agent) {
      return { name: "", description: "", system_prompt: "", tags: "" };
    }
    return {
      name: agent.name,
      description: agent.description || "",
      system_prompt: agent.system_prompt,
      tags: agent.tags.join(", "),
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

  // Capabilities state
  const initialCapabilities = useMemo(() => {
    if (!agentCapabilities) return [];
    return agentCapabilities
      .sort((a, b) => a.position - b.position)
      .map((ac) => ac.capability_id);
  }, [agentCapabilities]);

  const [localCapabilities, setLocalCapabilities] = useState<
    CapabilityId[] | null
  >(null);
  const selectedCapabilities = localCapabilities ?? initialCapabilities;

  // Capabilities handlers
  const handleToggle = useCallback(
    (capabilityId: CapabilityId, checked: boolean) => {
      const current = localCapabilities ?? initialCapabilities;
      let newSelected: CapabilityId[];
      if (checked) {
        newSelected = [...current, capabilityId];
      } else {
        newSelected = current.filter((id) => id !== capabilityId);
      }
      setLocalCapabilities(newSelected);
    },
    [localCapabilities, initialCapabilities]
  );

  const moveUp = useCallback(
    (index: number) => {
      if (index === 0) return;
      const current = localCapabilities ?? initialCapabilities;
      const newSelected = [...current];
      [newSelected[index - 1], newSelected[index]] = [
        newSelected[index],
        newSelected[index - 1],
      ];
      setLocalCapabilities(newSelected);
    },
    [localCapabilities, initialCapabilities]
  );

  const moveDown = useCallback(
    (index: number) => {
      const current = localCapabilities ?? initialCapabilities;
      if (index === current.length - 1) return;
      const newSelected = [...current];
      [newSelected[index], newSelected[index + 1]] = [
        newSelected[index + 1],
        newSelected[index],
      ];
      setLocalCapabilities(newSelected);
    },
    [localCapabilities, initialCapabilities]
  );

  const getCapabilityInfo = useCallback(
    (id: CapabilityId): Capability | undefined =>
      allCapabilities?.find((c) => c.id === id),
    [allCapabilities]
  );

  // Submit handler
  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();

    try {
      // Parse tags
      const tags = formData.tags
        .split(",")
        .map((t) => t.trim())
        .filter((t) => t.length > 0);

      // Update agent
      await updateAgent.mutateAsync({
        agentId,
        request: {
          name: formData.name,
          description: formData.description || undefined,
          system_prompt: formData.system_prompt,
          tags,
        },
      });

      // Update capabilities if changed
      const capabilitiesToSave = localCapabilities ?? initialCapabilities;
      if (
        JSON.stringify(capabilitiesToSave) !==
        JSON.stringify(initialCapabilities)
      ) {
        await setCapabilities.mutateAsync({
          agentId,
          request: { capabilities: capabilitiesToSave },
        });
      }

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

  const isLoading =
    agentLoading || capabilitiesLoading || agentCapabilitiesLoading;
  const isSaving = updateAgent.isPending || setCapabilities.isPending;

  if (isLoading) {
    return (
      <div className="container mx-auto p-6 max-w-4xl">
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

  // Filter capabilities
  const availableCapabilities =
    allCapabilities?.filter((c) => c.status === "available") || [];
  const comingSoonCapabilities =
    allCapabilities?.filter((c) => c.status === "coming_soon") || [];

  return (
    <div className="container mx-auto p-6 max-w-4xl">
      <Link
        href={`/agents/${agentId}`}
        className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-6"
      >
        <ArrowLeft className="w-4 h-4 mr-2" />
        Back to Agent
      </Link>

      <h1 className="text-2xl font-bold mb-6">Edit Agent</h1>

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
              <CardContent className="space-y-4">
                {/* Selected capabilities with reordering */}
                {selectedCapabilities.length > 0 && (
                  <div className="space-y-2">
                    <p className="text-sm font-medium text-muted-foreground">
                      Enabled ({selectedCapabilities.length})
                    </p>
                    {selectedCapabilities.map((capId, index) => {
                      const cap = getCapabilityInfo(capId);
                      if (!cap) return null;
                      const IconComponent = cap.icon
                        ? iconMap[cap.icon] || CircleOff
                        : CircleOff;

                      return (
                        <div
                          key={capId}
                          className="flex items-center gap-2 p-2 rounded-md border bg-muted/50"
                        >
                          <div className="flex flex-col gap-0.5">
                            <button
                              type="button"
                              onClick={() => moveUp(index)}
                              disabled={index === 0}
                              className="text-muted-foreground hover:text-foreground disabled:opacity-30 p-0.5"
                              aria-label="Move up"
                            >
                              <ChevronUp className="w-3 h-3" />
                            </button>
                            <button
                              type="button"
                              onClick={() => moveDown(index)}
                              disabled={index === selectedCapabilities.length - 1}
                              className="text-muted-foreground hover:text-foreground disabled:opacity-30 p-0.5"
                              aria-label="Move down"
                            >
                              <ChevronDown className="w-3 h-3" />
                            </button>
                          </div>
                          <span className="text-xs text-muted-foreground w-4">
                            {index + 1}
                          </span>
                          <IconComponent className="w-4 h-4" />
                          <span className="flex-1 text-sm">{cap.name}</span>
                          <Checkbox
                            checked={true}
                            onCheckedChange={(checked) =>
                              handleToggle(capId, checked as boolean)
                            }
                          />
                        </div>
                      );
                    })}
                  </div>
                )}

                {/* Available capabilities not yet selected */}
                {availableCapabilities.filter(
                  (c) => !selectedCapabilities.includes(c.id)
                ).length > 0 && (
                  <div className="space-y-2">
                    <p className="text-sm font-medium text-muted-foreground">
                      Available
                    </p>
                    {availableCapabilities
                      .filter((c) => !selectedCapabilities.includes(c.id))
                      .map((cap) => {
                        const IconComponent = cap.icon
                          ? iconMap[cap.icon] || CircleOff
                          : CircleOff;

                        return (
                          <div
                            key={cap.id}
                            className="flex items-center gap-2 p-2 rounded-md border hover:bg-muted/50"
                          >
                            <IconComponent className="w-4 h-4" />
                            <div className="flex-1">
                              <p className="text-sm">{cap.name}</p>
                              <p className="text-xs text-muted-foreground">
                                {cap.description}
                              </p>
                            </div>
                            <Checkbox
                              checked={false}
                              onCheckedChange={(checked) =>
                                handleToggle(cap.id, checked as boolean)
                              }
                            />
                          </div>
                        );
                      })}
                  </div>
                )}

                {/* Coming soon capabilities */}
                {comingSoonCapabilities.length > 0 && (
                  <div className="space-y-2">
                    <p className="text-sm font-medium text-muted-foreground">
                      Coming Soon
                    </p>
                    {comingSoonCapabilities.map((cap) => {
                      const IconComponent = cap.icon
                        ? iconMap[cap.icon] || CircleOff
                        : CircleOff;

                      return (
                        <div
                          key={cap.id}
                          className="flex items-center gap-2 p-2 rounded-md border opacity-60"
                        >
                          <IconComponent className="w-4 h-4" />
                          <div className="flex-1">
                            <p className="text-sm">{cap.name}</p>
                            <p className="text-xs text-muted-foreground">
                              {cap.description}
                            </p>
                          </div>
                          <Badge variant="secondary">Coming Soon</Badge>
                        </div>
                      );
                    })}
                  </div>
                )}

                {selectedCapabilities.length === 0 &&
                  availableCapabilities.length === 0 && (
                    <p className="text-center py-4 text-muted-foreground">
                      No capabilities available
                    </p>
                  )}
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

            {(updateAgent.error || setCapabilities.error) && (
              <p className="text-sm text-destructive">
                Error:{" "}
                {updateAgent.error?.message || setCapabilities.error?.message}
              </p>
            )}
          </div>
        </div>
      </form>
    </div>
  );
}
