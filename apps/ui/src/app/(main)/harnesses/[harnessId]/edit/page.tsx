"use client";

import { use, useState, useMemo, useCallback } from "react";
import { useRouter } from "next/navigation";
import Link from "next/link";
import {
  useHarness,
  useHarnesses,
  useUpdateHarness,
  useDeleteHarness,
  useDestroyHarness,
  useCapabilities,
} from "@/hooks";
import { usePolicies } from "@/hooks/use-policies";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { PromptEditor } from "@/components/ui/prompt-editor";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { CapabilitySelector } from "@/components/agents/capability-selector";
import { HarnessSelect } from "@/components/harness/harness-select";
import { HarnessPreview } from "@/components/harnesses/harness-preview";
import { InitialFilesEditor } from "@/components/initial-files-editor";
import { ModelPicker } from "@/components/models/model-picker";
import { ArrowLeft, Save, Trash2, Eye, Edit2 } from "lucide-react";
import type { AgentCapabilityConfig, InitialFile } from "@/lib/api/types";
import { isReadOnlyStatus } from "@/lib/entity-lifecycle";

interface FormData {
  name: string;
  description: string;
  system_prompt: string;
  parent_harness_id: string;
  tags: string;
  default_model_id: string;
}

export default function EditHarnessPage({ params }: { params: Promise<{ harnessId: string }> }) {
  const { harnessId } = use(params);
  const router = useRouter();

  const { data: harness, isLoading: harnessLoading } = useHarness(harnessId);
  const { data: harnesses = [] } = useHarnesses();
  const updateHarness = useUpdateHarness();
  const deleteHarness = useDeleteHarness();
  const destroyHarness = useDestroyHarness();
  const { can: canPolicies } = usePolicies("harnesses");

  const { data: allCapabilities, isLoading: capabilitiesLoading } = useCapabilities();

  const [activeTab, setActiveTab] = useState<string>("edit");
  const [showDeleteDialog, setShowDeleteDialog] = useState(false);
  const [formChanges, setFormChanges] = useState<Partial<FormData>>({});

  const initialFormData = useMemo((): FormData => {
    if (!harness) {
      return {
        name: "",
        description: "",
        system_prompt: "",
        parent_harness_id: "",
        tags: "",
        default_model_id: "",
      };
    }
    return {
      name: harness.name,
      description: harness.description || "",
      system_prompt: harness.system_prompt,
      parent_harness_id: harness.parent_harness_id || "",
      tags: harness.tags.join(", "),
      default_model_id: harness.default_model_id || "",
    };
  }, [harness]);

  const formData = useMemo(
    () => ({ ...initialFormData, ...formChanges }),
    [initialFormData, formChanges],
  );
  const selectedParentHarness = useMemo(
    () =>
      formData.parent_harness_id
        ? harnesses.find((candidate) => candidate.id === formData.parent_harness_id)
        : undefined,
    [formData.parent_harness_id, harnesses],
  );

  const handleFormChange = useCallback((field: keyof FormData, value: string) => {
    setFormChanges((prev) => ({ ...prev, [field]: value }));
  }, []);

  const initialCapabilities = useMemo(() => {
    return harness?.capabilities ?? [];
  }, [harness?.capabilities]);

  const [localCapabilities, setLocalCapabilities] = useState<AgentCapabilityConfig[] | null>(null);
  const selectedCapabilities = localCapabilities ?? initialCapabilities;
  const initialFiles = useMemo(() => harness?.initial_files ?? [], [harness?.initial_files]);
  const [localInitialFiles, setLocalInitialFiles] = useState<InitialFile[] | null>(null);
  const selectedInitialFiles = localInitialFiles ?? initialFiles;

  const handleCapabilitiesChange = useCallback((newCapabilities: AgentCapabilityConfig[]) => {
    setLocalCapabilities(newCapabilities);
  }, []);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();

    try {
      const tags = formData.tags
        .split(",")
        .map((t) => t.trim())
        .filter((t) => t.length > 0);

      const capabilitiesToSave = localCapabilities ?? initialCapabilities;
      const capabilitiesChanged =
        JSON.stringify(capabilitiesToSave) !== JSON.stringify(initialCapabilities);
      const initialFilesToSave = localInitialFiles ?? initialFiles;
      const initialFilesChanged =
        JSON.stringify(initialFilesToSave) !== JSON.stringify(initialFiles);

      await updateHarness.mutateAsync({
        harnessId,
        request: {
          name: formData.name,
          description: formData.description || undefined,
          system_prompt: formData.system_prompt,
          parent_harness_id: formData.parent_harness_id || null,
          tags,
          default_model_id: formData.default_model_id || undefined,
          ...(capabilitiesChanged && { capabilities: capabilitiesToSave }),
          ...(initialFilesChanged && { initial_files: initialFilesToSave }),
        },
      });

      router.push(`/harnesses/${harnessId}`);
    } catch (error) {
      console.error("Failed to update harness:", error);
    }
  };

  const handleArchive = async () => {
    try {
      await deleteHarness.mutateAsync(harnessId);
    } catch (error) {
      console.error("Failed to archive harness:", error);
    }
  };

  const handleDestroy = async () => {
    try {
      await destroyHarness.mutateAsync(harnessId);
      router.push("/harnesses");
    } catch (error) {
      console.error("Failed to delete harness:", error);
    }
  };

  const isLoading = harnessLoading || capabilitiesLoading;
  const isSaving = updateHarness.isPending;
  const isReadOnly = isReadOnlyStatus(harness?.status);
  const canDangerousDelete = canPolicies("harness.dangerous");

  // Built-in harnesses cannot be edited — redirect to detail page
  if (harness?.is_built_in) {
    router.replace(`/harnesses/${harnessId}`);
    return null;
  }

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

  if (!harness) {
    return (
      <div className="container mx-auto p-6">
        <div className="text-red-500">Harness not found</div>
        <Link href="/harnesses" className="text-blue-500 hover:underline">
          Back to harnesses
        </Link>
      </div>
    );
  }

  return (
    <div className="container mx-auto p-6">
      <Link
        href={`/harnesses/${harnessId}`}
        className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-6"
      >
        <ArrowLeft className="w-4 h-4 mr-2" />
        Back to Harness
      </Link>

      <div className="flex items-center justify-between mb-6">
        <h1 className="text-2xl font-bold">Edit Harness</h1>
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
                    <CardTitle>Harness Details</CardTitle>
                  </CardHeader>
                  <CardContent className="space-y-6">
                    <div className="space-y-2">
                      <Label htmlFor="name">Name</Label>
                      <Input
                        id="name"
                        placeholder="My Harness"
                        value={formData.name}
                        onChange={(e) => handleFormChange("name", e.target.value)}
                        disabled={isSaving || isReadOnly}
                        required
                      />
                    </div>

                    <div className="space-y-2">
                      <Label htmlFor="description">Description</Label>
                      <Textarea
                        id="description"
                        placeholder="Describe what this harness does..."
                        value={formData.description}
                        onChange={(e) => handleFormChange("description", e.target.value)}
                        disabled={isSaving || isReadOnly}
                        rows={2}
                      />
                    </div>

                    <div className="space-y-2">
                      <Label htmlFor="parent-harness">Parent Harness</Label>
                      <HarnessSelect
                        value={formData.parent_harness_id}
                        onValueChange={(value) => handleFormChange("parent_harness_id", value)}
                        placeholder="No parent harness"
                        includeNoneOption
                        noneLabel="No parent harness"
                        excludeIds={[harnessId]}
                        disabled={isSaving || isReadOnly}
                      />
                      <p className="text-xs text-muted-foreground">
                        Inherit prompt, capabilities, initial files, and model fallback from a
                        parent harness.
                      </p>
                    </div>

                    <div className="space-y-2">
                      <Label htmlFor="tags">Tags</Label>
                      <Input
                        id="tags"
                        placeholder="tag1, tag2, tag3"
                        value={formData.tags}
                        onChange={(e) => handleFormChange("tags", e.target.value)}
                        disabled={isSaving || isReadOnly}
                      />
                      <p className="text-xs text-muted-foreground">Comma-separated list of tags</p>
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
                      <p className="text-xs text-muted-foreground">
                        Instructions for the AI model (supports Markdown)
                      </p>
                    </div>

                    <InitialFilesEditor
                      value={selectedInitialFiles}
                      onChange={setLocalInitialFiles}
                      disabled={isSaving || isReadOnly}
                      description="Files copied into each new session created from this harness."
                    />
                  </CardContent>
                </Card>

                {/* Danger Zone */}
                <Card className="border-destructive/50">
                  <CardHeader>
                    <CardTitle className="text-destructive">Danger Zone</CardTitle>
                    <CardDescription>Irreversible actions that affect this harness</CardDescription>
                  </CardHeader>
                  <CardContent>
                    <div className="flex items-center justify-between">
                      <div>
                        <p className="font-medium">
                          {harness.status === "archived"
                            ? "Delete this harness"
                            : "Archive this harness"}
                        </p>
                        <p className="text-sm text-muted-foreground">
                          {harness.status === "archived"
                            ? "Permanently delete this archived harness. Existing references will render as deleted tombstones."
                            : "Archive this harness. It will stay visible when archived items are shown, become read-only, and stop being assignable."}
                        </p>
                      </div>
                      {harness.status === "archived" ? (
                        canDangerousDelete && (
                          <Button
                            type="button"
                            variant="destructive"
                            onClick={() => setShowDeleteDialog(true)}
                            disabled={destroyHarness.isPending}
                          >
                            <Trash2 className="w-4 h-4 mr-2" />
                            {destroyHarness.isPending ? "Deleting..." : "Delete Harness"}
                          </Button>
                        )
                      ) : (
                        <Button
                          type="button"
                          variant="outline"
                          onClick={handleArchive}
                          disabled={deleteHarness.isPending}
                        >
                          {deleteHarness.isPending ? "Archiving..." : "Archive Harness"}
                        </Button>
                      )}
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
                      disabled={isSaving || isReadOnly}
                    />
                  </CardContent>
                </Card>

                {/* Save button */}
                <div className="flex gap-4">
                  <Button type="submit" disabled={isSaving || isReadOnly} className="flex-1">
                    <Save className="w-4 h-4 mr-2" />
                    {isReadOnly
                      ? "Archived Harnesses Are Read-Only"
                      : isSaving
                        ? "Saving..."
                        : "Save Changes"}
                  </Button>
                  <Button type="button" variant="outline" onClick={() => router.back()}>
                    Cancel
                  </Button>
                </div>

                {updateHarness.error && (
                  <p className="text-sm text-destructive">Error: {updateHarness.error.message}</p>
                )}
              </div>
            </div>
          </form>
        </TabsContent>

        {/* Preview Tab Content */}
        <TabsContent value="preview">
          <div className="grid gap-6 lg:grid-cols-3">
            <div className="lg:col-span-2">
              <HarnessPreview
                systemPrompt={formData.system_prompt}
                capabilities={selectedCapabilities}
                parentHarnessId={formData.parent_harness_id || undefined}
              />
            </div>

            {/* Summary sidebar */}
            <div className="space-y-6">
              <Card>
                <CardHeader>
                  <CardTitle>Harness Summary</CardTitle>
                  <CardDescription>Current configuration</CardDescription>
                </CardHeader>
                <CardContent className="space-y-4">
                  <div>
                    <p className="text-sm font-medium">Name</p>
                    <p className="text-sm text-muted-foreground">{formData.name || "(not set)"}</p>
                  </div>
                  <div>
                    <p className="text-sm font-medium">Description</p>
                    <p className="text-sm text-muted-foreground">
                      {formData.description || "(not set)"}
                    </p>
                  </div>
                  <div>
                    <p className="text-sm font-medium">Parent Harness</p>
                    <p className="text-sm text-muted-foreground">
                      {(selectedParentHarness?.name ?? formData.parent_harness_id) || "(none)"}
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
                    This preview shows what the final harness will look like after applying all
                    capabilities. Switch to the Edit tab to make changes.
                  </p>
                </CardContent>
              </Card>
            </div>
          </div>
        </TabsContent>
      </Tabs>
      <Dialog open={showDeleteDialog} onOpenChange={setShowDeleteDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete Harness</DialogTitle>
            <DialogDescription>
              Permanently delete the archived harness &quot;{harness?.name}&quot;? Existing
              references will render as deleted tombstones.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setShowDeleteDialog(false)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={handleDestroy}
              disabled={destroyHarness.isPending}
            >
              {destroyHarness.isPending ? "Deleting..." : "Delete Harness"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
