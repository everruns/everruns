"use client";

import { use, useState, useMemo, useCallback } from "react";
import { useRouter } from "next/navigation";
import {
  useAgent,
  useUpdateAgent,
  useDeleteAgent,
  useDestroyAgent,
  useCapabilities,
  useAgentNameAvailability,
  usePageTitle,
} from "@/hooks";
import { usePolicies } from "@/hooks/use-policies";
import { ResourceNotFound } from "@/components/resource-not-found";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { TagInput } from "@/components/ui/tag-input";
import { PromptEditor } from "@/components/ui/prompt-editor";
import { Skeleton } from "@/components/ui/skeleton";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  PageControlStrip,
  SectionTabs,
  PageColumns,
  PageMain,
  PageRail,
  PageFooter,
  BackLink,
} from "@/components/layout";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { CapabilitySelector } from "@/components/agents/capability-selector";
import { normalizeCapabilityConfigs } from "@/components/agents/capability-config";
import { AgentPreview } from "@/components/agents/agent-preview";
import { AgentChecks, applyByteSpanReplacement } from "@/components/agents/agent-checks";
import { AgentHealthCheck } from "@/components/agents/agent-health-check";
import { EntityDeleteErrorNotice } from "@/components/entity-delete-error-notice";
import { InitialFilesEditor } from "@/components/initial-files-editor";
import { NetworkAccessEditor, normalizeNetworkAccess } from "@/components/network-access-editor";
import { ModelPicker } from "@/components/models/model-picker";
import { HarnessSelect } from "@/components/harness/harness-select";
import { Trash2, Eye, Edit2, Check, X, Loader2, Boxes, Pencil } from "lucide-react";
import {
  agentFormSchema,
  getFieldErrors,
  type FieldErrors,
  parseTagList,
} from "@/lib/form-validation";
import type { AgentCapabilityConfig, InitialFile, NetworkAccessList } from "@/lib/api/types";
import { getDisplayName, isReadOnlyStatus } from "@/lib/entity-lifecycle";
import { joinTags } from "@/lib/tags";

interface FormData {
  display_name: string;
  name: string;
  description: string;
  system_prompt: string;
  tags: string;
  harness_id: string;
  default_model_id: string;
}

export default function EditAgentPage({ params }: { params: Promise<{ agentId: string }> }) {
  const { agentId } = use(params);
  const router = useRouter();

  // Agent data
  const { data: agent, isLoading: agentLoading } = useAgent(agentId);
  usePageTitle("Edit", agent ? getDisplayName(agent) : null, "Agent");
  const updateAgent = useUpdateAgent();
  const deleteAgent = useDeleteAgent();
  const destroyAgent = useDestroyAgent();
  const { can: canPolicies } = usePolicies("agents");

  // Capabilities data
  const { data: allCapabilities, isLoading: capabilitiesLoading } = useCapabilities();

  // Tab state
  const [activeTab, setActiveTab] = useState<string>("edit");
  const [showDeleteDialog, setShowDeleteDialog] = useState(false);
  const [fieldErrors, setFieldErrors] = useState<FieldErrors>({});

  // Form state - track user changes separately from initial values
  const [formChanges, setFormChanges] = useState<Partial<FormData>>({});

  // Compute initial values from agent data
  const initialFormData = useMemo((): FormData => {
    if (!agent) {
      return {
        display_name: "",
        name: "",
        description: "",
        system_prompt: "",
        tags: "",
        harness_id: "",
        default_model_id: "",
      };
    }
    return {
      display_name: agent.display_name || "",
      name: agent.name,
      description: agent.description || "",
      system_prompt: agent.system_prompt,
      tags: joinTags(agent.tags),
      harness_id: agent.harness_id || "",
      default_model_id: agent.default_model_id || "",
    };
  }, [agent]);

  // Merge initial values with user changes
  const formData = useMemo(
    () => ({ ...initialFormData, ...formChanges }),
    [initialFormData, formChanges],
  );

  const handleFormChange = useCallback((field: keyof FormData, value: string) => {
    setFormChanges((prev) => ({ ...prev, [field]: value }));
    setFieldErrors((prev) => ({ ...prev, [field]: undefined }));
  }, []);

  // Only check availability when name has been changed from original
  const nameChanged = formData.name !== initialFormData.name;
  const nameAvailability = useAgentNameAvailability(nameChanged ? formData.name : "", agentId);

  // Capabilities state - now included directly in agent response
  // Use full AgentCapabilityConfig objects to preserve per-agent config
  const initialCapabilities = useMemo(() => {
    return normalizeCapabilityConfigs(agent?.capabilities);
  }, [agent?.capabilities]);

  const [localCapabilities, setLocalCapabilities] = useState<AgentCapabilityConfig[] | null>(null);
  const selectedCapabilities = localCapabilities ?? initialCapabilities;
  const initialFiles = useMemo(() => agent?.initial_files ?? [], [agent?.initial_files]);
  const [localInitialFiles, setLocalInitialFiles] = useState<InitialFile[] | null>(null);
  const selectedInitialFiles = localInitialFiles ?? initialFiles;
  const initialNetworkAccess = agent?.network_access ?? null;
  const [localNetworkAccess, setLocalNetworkAccess] = useState<NetworkAccessList | null>(null);

  // Capabilities change handler
  const handleCapabilitiesChange = useCallback((newCapabilities: AgentCapabilityConfig[]) => {
    setLocalCapabilities(newCapabilities);
  }, []);

  // Submit handler
  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const parsed = agentFormSchema.safeParse(formData);
    if (!parsed.success) {
      setFieldErrors(getFieldErrors(parsed.error));
      return;
    }

    try {
      // Parse tags
      const tags = parseTagList(parsed.data.tags);

      // Get capabilities to save (already full AgentCapabilityConfig format)
      const capabilitiesToSave = localCapabilities ?? initialCapabilities;
      const capabilitiesChanged =
        JSON.stringify(capabilitiesToSave) !== JSON.stringify(initialCapabilities);
      const initialFilesToSave = localInitialFiles ?? initialFiles;
      const initialFilesChanged =
        JSON.stringify(initialFilesToSave) !== JSON.stringify(initialFiles);
      // Omitting network_access leaves it unchanged; {} clears the layer.
      const networkAccessChanged =
        localNetworkAccess !== null &&
        JSON.stringify(normalizeNetworkAccess(localNetworkAccess)) !==
          JSON.stringify(normalizeNetworkAccess(initialNetworkAccess));

      // Update agent (capabilities are now part of the agent resource)
      await updateAgent.mutateAsync({
        agentId,
        request: {
          name: parsed.data.name,
          display_name: parsed.data.display_name,
          description: parsed.data.description,
          system_prompt: parsed.data.system_prompt,
          tags,
          harness_id: parsed.data.harness_id,
          default_model_id: parsed.data.default_model_id,
          // Only include capabilities if they changed
          ...(capabilitiesChanged && { capabilities: capabilitiesToSave }),
          ...(initialFilesChanged && { initial_files: initialFilesToSave }),
          ...(networkAccessChanged && { network_access: localNetworkAccess }),
        },
      });

      router.push(`/agents/${agentId}`);
    } catch (error) {
      console.error("Failed to update agent:", error);
    }
  };

  // Delete handler
  const handleArchive = async () => {
    try {
      await deleteAgent.mutateAsync(agentId);
    } catch (error) {
      console.error("Failed to archive agent:", error);
    }
  };

  const handleDestroy = async () => {
    try {
      await destroyAgent.mutateAsync(agentId);
      router.push("/agents");
    } catch (error) {
      console.error("Failed to delete agent:", error);
    }
  };

  const isLoading = agentLoading || capabilitiesLoading;
  const isSaving = updateAgent.isPending;
  const isReadOnly = isReadOnlyStatus(agent?.status);
  const canDangerousDelete = canPolicies("agent.dangerous");
  const deleteError = deleteAgent.error ?? destroyAgent.error;
  const deleteAction = agent?.status === "archived" ? "delete" : "archive";

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
      <ResourceNotFound
        title="Agent not found"
        description="This agent may have been deleted, moved to another organization, or the URL may be wrong."
        backHref="/agents"
        backLabel="Back to agents"
        resourceId={agentId}
      />
    );
  }

  const agentDisplayName = getDisplayName(agent);

  return (
    <PageContainer>
      <PageBreadcrumb
        items={[
          { label: "Agents", href: "/agents" },
          { label: agentDisplayName, href: `/agents/${agentId}` },
          { label: "Edit" },
        ]}
      />

      <PageMasthead
        icon={<Boxes />}
        title={agentDisplayName}
        badges={
          <Badge variant="accent">
            <Pencil className="size-3" />
            Editing
          </Badge>
        }
        description="Changes apply to new sessions only. Running sessions keep the current definition."
        meta={
          <>
            <span>
              Identity <span className="font-mono text-primary">{agent.name}</span>
            </span>
            <span>
              Last edited{" "}
              <span className="text-foreground">
                {new Date(agent.updated_at).toLocaleDateString()}
              </span>
            </span>
          </>
        }
        actions={
          <>
            <Button type="submit" form="agent-edit-form" disabled={isSaving || isReadOnly}>
              <Check className="size-4" />
              {isReadOnly ? "Read-only" : isSaving ? "Saving..." : "Save changes"}
            </Button>
            <Button type="button" variant="outline" onClick={() => router.back()}>
              Discard
            </Button>
          </>
        }
      />

      <PageControlStrip>
        <SectionTabs
          value={activeTab}
          onValueChange={setActiveTab}
          items={[
            { value: "edit", label: "Edit", icon: <Edit2 className="size-4" /> },
            { value: "preview", label: "Preview", icon: <Eye className="size-4" /> },
          ]}
        />
      </PageControlStrip>

      {activeTab === "edit" && (
        <form id="agent-edit-form" onSubmit={handleSubmit}>
          <PageColumns>
            {/* Main form */}
            <PageMain>
              <AgentChecks
                systemPrompt={formData.system_prompt}
                capabilities={selectedCapabilities}
                tools={agent.tools ?? []}
                onApplyFix={(start, end, replacement) =>
                  handleFormChange(
                    "system_prompt",
                    applyByteSpanReplacement(formData.system_prompt, start, end, replacement),
                  )
                }
              />

              <AgentHealthCheck agentId={agent.id} />

              <Card>
                <CardHeader>
                  <CardTitle>Agent Details</CardTitle>
                </CardHeader>
                <CardContent className="space-y-6">
                  <div className="space-y-2">
                    <Label htmlFor="name">Name</Label>
                    <Input
                      id="name"
                      placeholder="customer-support"
                      value={formData.name}
                      onChange={(e) => handleFormChange("name", e.target.value)}
                      aria-invalid={!!fieldErrors.name}
                      disabled={isSaving || isReadOnly}
                      required
                    />
                    {fieldErrors.name && (
                      <p className="text-xs text-destructive">{fieldErrors.name}</p>
                    )}
                    {nameChanged && formData.name.length >= 2 && (
                      <div className="flex items-center gap-1.5 text-xs">
                        {nameAvailability.isChecking ? (
                          <>
                            <Loader2 className="w-3 h-3 animate-spin text-muted-foreground" />
                            <span className="text-muted-foreground">Checking availability…</span>
                          </>
                        ) : nameAvailability.available === true ? (
                          <>
                            <Check className="w-3 h-3 text-green-600" />
                            <span className="text-green-600">Name is available</span>
                          </>
                        ) : nameAvailability.available === false ? (
                          <>
                            <X className="w-3 h-3 text-destructive" />
                            <span className="text-destructive">
                              Name is already taken or invalid
                            </span>
                          </>
                        ) : null}
                      </div>
                    )}
                    <p className="text-xs text-muted-foreground">
                      Unique identifier used in URLs and API. Lowercase letters, numbers, and
                      hyphens.
                    </p>
                  </div>

                  <div className="space-y-2">
                    <Label htmlFor="display_name">Display Name</Label>
                    <Input
                      id="display_name"
                      placeholder={formData.name ? undefined : "Customer Support Agent"}
                      value={formData.display_name}
                      onChange={(e) => handleFormChange("display_name", e.target.value)}
                      disabled={isSaving || isReadOnly}
                    />
                    <p className="text-xs text-muted-foreground">
                      Optional human-readable label shown in the UI. Defaults to name if empty.
                    </p>
                  </div>

                  <div className="space-y-2">
                    <Label htmlFor="description">Description</Label>
                    <Textarea
                      id="description"
                      placeholder="Describe what this agent does..."
                      value={formData.description}
                      onChange={(e) => handleFormChange("description", e.target.value)}
                      disabled={isSaving || isReadOnly}
                      rows={2}
                    />
                  </div>

                  <div className="space-y-2">
                    <Label htmlFor="tags">Tags</Label>
                    <TagInput
                      id="tags"
                      placeholder="Add a tag…"
                      value={formData.tags}
                      onChange={(value) => handleFormChange("tags", value)}
                      disabled={isSaving || isReadOnly}
                      aria-describedby="tags-help"
                    />
                    <p id="tags-help" className="text-xs text-muted-foreground">
                      Press Enter or comma to add a tag. Backspace removes the last tag.
                    </p>
                  </div>

                  <div className="space-y-2">
                    <Label htmlFor="harness">
                      Harness <span className="text-destructive">*</span>
                    </Label>
                    <HarnessSelect
                      id="harness"
                      value={formData.harness_id}
                      onValueChange={(value) => handleFormChange("harness_id", value)}
                      disabled={isSaving || isReadOnly}
                      placeholder="Select a harness"
                    />
                    {fieldErrors.harness_id && (
                      <p className="text-xs text-destructive">{fieldErrors.harness_id}</p>
                    )}
                    <p className="text-xs text-muted-foreground">
                      The base execution harness this agent runs on. Sessions started from this
                      agent inherit it.
                    </p>
                  </div>

                  <div className="space-y-2">
                    <Label htmlFor="model">Model (optional)</Label>
                    <ModelPicker
                      value={formData.default_model_id || ""}
                      onChange={(value) => handleFormChange("default_model_id", value)}
                      disabled={isSaving || isReadOnly}
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
                      onChange={(value) => handleFormChange("system_prompt", value)}
                      disabled={isSaving || isReadOnly}
                      required
                    />
                    {fieldErrors.system_prompt && (
                      <p className="text-xs text-destructive">{fieldErrors.system_prompt}</p>
                    )}
                    <p className="text-xs text-muted-foreground">
                      Instructions for the AI model (supports Markdown)
                    </p>
                  </div>

                  <InitialFilesEditor
                    value={selectedInitialFiles}
                    onChange={setLocalInitialFiles}
                    disabled={isSaving || isReadOnly}
                    description="Files copied into each new session for this agent."
                  />

                  <NetworkAccessEditor
                    value={initialNetworkAccess}
                    onChange={setLocalNetworkAccess}
                    disabled={isSaving || isReadOnly}
                    description="Control which hosts this agent's sessions can reach via network-capable tools. Narrows the harness policy; sessions can narrow it further."
                  />
                </CardContent>
              </Card>

              {/* Danger Zone */}
              <Card className="border-destructive/50">
                <CardHeader>
                  <CardTitle className="text-destructive">Danger Zone</CardTitle>
                  <CardDescription>Irreversible actions that affect this agent</CardDescription>
                </CardHeader>
                <CardContent>
                  <div className="flex items-center justify-between">
                    <div>
                      <p className="font-medium">
                        {agent.status === "archived" ? "Delete this agent" : "Archive this agent"}
                      </p>
                      <p className="text-sm text-muted-foreground">
                        {agent.status === "archived"
                          ? "Permanently delete this archived agent. Existing references will render as deleted tombstones."
                          : "Archive this agent. It will stay visible when archived items are shown, become read-only, and stop being assignable."}
                      </p>
                    </div>
                    {agent.status === "archived" ? (
                      canDangerousDelete && (
                        <Button
                          type="button"
                          variant="destructive"
                          onClick={() => setShowDeleteDialog(true)}
                          disabled={destroyAgent.isPending}
                        >
                          <Trash2 className="w-4 h-4 mr-2" />
                          {destroyAgent.isPending ? "Deleting..." : "Delete Agent"}
                        </Button>
                      )
                    ) : (
                      <Button
                        type="button"
                        variant="outline"
                        onClick={handleArchive}
                        disabled={deleteAgent.isPending}
                      >
                        {deleteAgent.isPending ? "Archiving..." : "Archive Agent"}
                      </Button>
                    )}
                  </div>
                  {deleteError && (
                    <EntityDeleteErrorNotice
                      entityKind="agent"
                      action={deleteAction}
                      message={deleteError.message}
                      className="mt-4"
                    />
                  )}
                </CardContent>
              </Card>
            </PageMain>

            {/* Capabilities sidebar */}
            <PageRail>
              <Card>
                <CardHeader>
                  <CardTitle>Capabilities</CardTitle>
                </CardHeader>
                <CardContent>
                  <CapabilitySelector
                    capabilities={allCapabilities || []}
                    selected={selectedCapabilities}
                    onChange={handleCapabilitiesChange}
                    disabled={isSaving || isReadOnly}
                  />
                </CardContent>
              </Card>

              {updateAgent.error && (
                <p className="text-sm text-destructive">Error: {updateAgent.error.message}</p>
              )}
            </PageRail>
          </PageColumns>
        </form>
      )}

      {/* Preview Tab Content */}
      {activeTab === "preview" && (
        <div className="grid gap-6 lg:grid-cols-3">
          <div className="lg:col-span-2">
            <AgentPreview
              systemPrompt={formData.system_prompt}
              capabilities={selectedCapabilities}
              initialFiles={selectedInitialFiles}
              tools={agent.tools ?? []}
            />
          </div>

          {/* Summary sidebar */}
          <div className="space-y-6">
            <Card>
              <CardHeader>
                <CardTitle>Agent Summary</CardTitle>
                <CardDescription>Current configuration</CardDescription>
              </CardHeader>
              <CardContent className="space-y-4">
                <div>
                  <p className="text-sm font-medium">Name</p>
                  <p className="text-sm text-muted-foreground font-mono">
                    {formData.name || "(not set)"}
                  </p>
                </div>
                <div>
                  <p className="text-sm font-medium">Display Name</p>
                  <p className="text-sm text-muted-foreground">
                    {formData.display_name || "(not set)"}
                  </p>
                </div>
                <div>
                  <p className="text-sm font-medium">Description</p>
                  <p className="text-sm text-muted-foreground">
                    {formData.description || "(not set)"}
                  </p>
                </div>
                <div>
                  <p className="text-sm font-medium">Capabilities</p>
                  <p className="text-sm text-muted-foreground">
                    {selectedCapabilities.length} capabilit
                    {selectedCapabilities.length !== 1 ? "ies" : "y"} enabled
                  </p>
                </div>
              </CardContent>
            </Card>

            <Card className="border-dashed">
              <CardContent className="pt-6">
                <p className="text-sm text-muted-foreground text-center">
                  This preview shows what the final agent will look like after applying all
                  capabilities. Switch to the Edit tab to make changes.
                </p>
              </CardContent>
            </Card>
          </div>
        </div>
      )}

      <PageFooter>
        <BackLink href={`/agents/${agentId}`}>Back to {agentDisplayName}</BackLink>
      </PageFooter>

      <Dialog open={showDeleteDialog} onOpenChange={setShowDeleteDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete Agent</DialogTitle>
            <DialogDescription>
              Permanently delete the archived agent &quot;{agentDisplayName}
              &quot;? Existing references will render as deleted tombstones.
            </DialogDescription>
            {destroyAgent.error && (
              <EntityDeleteErrorNotice
                entityKind="agent"
                action="delete"
                message={destroyAgent.error.message}
                className="mt-4"
              />
            )}
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setShowDeleteDialog(false)}>
              Cancel
            </Button>
            <Button variant="destructive" onClick={handleDestroy} disabled={destroyAgent.isPending}>
              {destroyAgent.isPending ? "Deleting..." : "Delete Agent"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </PageContainer>
  );
}
