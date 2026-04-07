"use client";

import { useState, useCallback } from "react";
import { useRouter } from "next/navigation";
import { useCreateHarness, useCapabilities, useHarnessNameAvailability } from "@/hooks";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { PromptEditor } from "@/components/ui/prompt-editor";
import Link from "next/link";
import { ArrowLeft, Check, X, Loader2 } from "lucide-react";
import { ModelPicker } from "@/components/models/model-picker";
import { CapabilitySelector } from "@/components/agents/capability-selector";
import { HarnessSelect } from "@/components/harness/harness-select";
import { InitialFilesEditor } from "@/components/initial-files-editor";
import type { AgentCapabilityConfig, InitialFile } from "@/lib/api/types";

export default function NewHarnessPage() {
  const router = useRouter();
  const createHarness = useCreateHarness();
  const { data: allCapabilities = [] } = useCapabilities();

  const [formData, setFormData] = useState({
    name: "",
    description: "",
    system_prompt: "",
    parent_harness_id: "",
    default_model_id: "",
    tags: "",
  });

  const nameAvailability = useHarnessNameAvailability(formData.name);

  const [selectedCapabilities, setSelectedCapabilities] = useState<AgentCapabilityConfig[]>([]);
  const [initialFiles, setInitialFiles] = useState<InitialFile[]>([]);

  const handleCapabilitiesChange = useCallback((capabilities: AgentCapabilityConfig[]) => {
    setSelectedCapabilities(capabilities);
  }, []);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();

    try {
      const tags = formData.tags
        .split(",")
        .map((t) => t.trim())
        .filter((t) => t.length > 0);

      const harness = await createHarness.mutateAsync({
        name: formData.name,
        description: formData.description || undefined,
        system_prompt: formData.system_prompt,
        parent_harness_id: formData.parent_harness_id || undefined,
        default_model_id: formData.default_model_id || undefined,
        tags: tags.length > 0 ? tags : undefined,
        capabilities: selectedCapabilities.length > 0 ? selectedCapabilities : undefined,
        initial_files: initialFiles.length > 0 ? initialFiles : undefined,
      });

      router.push(`/harnesses/${harness.id}`);
    } catch (error) {
      console.error("Failed to create harness:", error);
    }
  };

  return (
    <div className="container mx-auto p-6">
      <Link
        href="/harnesses"
        className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-6"
      >
        <ArrowLeft className="w-4 h-4 mr-2" />
        Back to Harnesses
      </Link>

      <Card>
        <CardHeader>
          <CardTitle>Create New Harness</CardTitle>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleSubmit} className="space-y-6">
            <div className="space-y-2">
              <Label htmlFor="name">Name</Label>
              <Input
                id="name"
                placeholder="my-harness"
                value={formData.name}
                onChange={(e) => setFormData({ ...formData, name: e.target.value })}
                required
              />
              {formData.name.length >= 2 && (
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
                      <span className="text-destructive">Name is already taken or invalid</span>
                    </>
                  ) : null}
                </div>
              )}
            </div>

            <div className="space-y-2">
              <Label htmlFor="description">Description</Label>
              <Textarea
                id="description"
                placeholder="Describe what this harness does..."
                value={formData.description}
                onChange={(e) => setFormData({ ...formData, description: e.target.value })}
                rows={2}
              />
            </div>

            <div className="space-y-2">
              <Label htmlFor="tags">Tags</Label>
              <Input
                id="tags"
                placeholder="tag1, tag2, tag3"
                value={formData.tags}
                onChange={(e) => setFormData({ ...formData, tags: e.target.value })}
              />
              <p className="text-xs text-muted-foreground">Comma-separated list of tags</p>
            </div>

            <div className="space-y-2">
              <Label htmlFor="parent-harness">Parent Harness</Label>
              <HarnessSelect
                value={formData.parent_harness_id}
                onValueChange={(value) => setFormData({ ...formData, parent_harness_id: value })}
                placeholder="No parent harness"
                includeNoneOption
                noneLabel="No parent harness"
                disabled={createHarness.isPending}
              />
              <p className="text-xs text-muted-foreground">
                Inherit system prompt, capabilities, initial files, and default model from another
                harness.
              </p>
            </div>

            <div className="space-y-2">
              <Label htmlFor="model">Model (optional)</Label>
              <ModelPicker
                value={formData.default_model_id}
                onChange={(value) => setFormData({ ...formData, default_model_id: value })}
                placeholder="Use default model"
              />
              <p className="text-xs text-muted-foreground">
                Select a specific model or leave empty to use the default
              </p>
            </div>

            <div className="space-y-2">
              <Label>Capabilities (optional)</Label>
              <CapabilitySelector
                capabilities={allCapabilities}
                selected={selectedCapabilities}
                onChange={handleCapabilitiesChange}
                disabled={createHarness.isPending}
                label=""
              />
              <p className="text-xs text-muted-foreground">
                Add capabilities to give your harness tools and specialized behaviors
              </p>
            </div>

            <div className="space-y-2">
              <Label htmlFor="system_prompt">System Prompt</Label>
              <PromptEditor
                id="system_prompt"
                placeholder="You are a helpful assistant..."
                value={formData.system_prompt}
                onChange={(value) => setFormData({ ...formData, system_prompt: value })}
                required
              />
              <p className="text-xs text-muted-foreground">
                Instructions for the AI model (supports Markdown)
              </p>
            </div>

            <InitialFilesEditor
              value={initialFiles}
              onChange={setInitialFiles}
              disabled={createHarness.isPending}
              description="Seed starter files into every session created from this harness."
            />

            <div className="flex gap-4">
              <Button type="submit" disabled={createHarness.isPending}>
                {createHarness.isPending ? "Creating..." : "Create Harness"}
              </Button>
              <Button type="button" variant="outline" onClick={() => router.back()}>
                Cancel
              </Button>
            </div>

            {createHarness.error && (
              <p className="text-sm text-destructive">Error: {createHarness.error.message}</p>
            )}
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
