"use client";

import { useState, useCallback } from "react";
import { useRouter } from "next/navigation";
import {
  useCreateAgent,
  useCapabilities,
  useAgentNameAvailability,
} from "@/hooks";
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
import { InitialFilesEditor } from "@/components/initial-files-editor";
import type { AgentCapabilityConfig, InitialFile } from "@/lib/api/types";

/** Convert a display name to a slug: lowercase, non-alphanumeric → hyphens, deduplicate, trim. */
function slugify(value: string): string {
  return value
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .replace(/-{2,}/g, "-");
}

export default function NewAgentPage() {
  const router = useRouter();
  const createAgent = useCreateAgent();
  const { data: allCapabilities = [] } = useCapabilities();

  const [formData, setFormData] = useState({
    display_name: "",
    name: "",
    description: "",
    system_prompt: "",
    default_model_id: "",
  });

  // Track whether the user has manually edited the slug
  const [nameManuallyEdited, setNameManuallyEdited] = useState(false);

  const nameAvailability = useAgentNameAvailability(formData.name);

  const [selectedCapabilities, setSelectedCapabilities] = useState<
    AgentCapabilityConfig[]
  >([]);
  const [initialFiles, setInitialFiles] = useState<InitialFile[]>([]);

  const handleCapabilitiesChange = useCallback(
    (capabilities: AgentCapabilityConfig[]) => {
      setSelectedCapabilities(capabilities);
    },
    [],
  );

  const handleDisplayNameChange = (value: string) => {
    const updates: Partial<typeof formData> = { display_name: value };
    // Auto-derive slug unless user has manually edited it
    if (!nameManuallyEdited) {
      updates.name = slugify(value);
    }
    setFormData((prev) => ({ ...prev, ...updates }));
  };

  const handleNameChange = (value: string) => {
    setNameManuallyEdited(true);
    setFormData((prev) => ({ ...prev, name: value }));
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();

    try {
      const agent = await createAgent.mutateAsync({
        name: formData.name,
        display_name: formData.display_name || undefined,
        description: formData.description || undefined,
        system_prompt: formData.system_prompt,
        default_model_id: formData.default_model_id || undefined,
        capabilities:
          selectedCapabilities.length > 0 ? selectedCapabilities : undefined,
        initial_files: initialFiles.length > 0 ? initialFiles : undefined,
      });

      router.push(`/agents/${agent.id}`);
    } catch (error) {
      console.error("Failed to create agent:", error);
    }
  };

  return (
    <div className="container mx-auto p-6">
      <Link
        href="/agents"
        className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-6"
      >
        <ArrowLeft className="w-4 h-4 mr-2" />
        Back to Agents
      </Link>

      <Card>
        <CardHeader>
          <CardTitle>Create New Agent</CardTitle>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleSubmit} className="space-y-6">
            <div className="space-y-2">
              <Label htmlFor="display_name">Display Name</Label>
              <Input
                id="display_name"
                placeholder="Customer Support Agent"
                value={formData.display_name}
                onChange={(e) => handleDisplayNameChange(e.target.value)}
              />
              <p className="text-xs text-muted-foreground">
                Human-readable name shown in the UI
              </p>
            </div>

            <div className="space-y-2">
              <Label htmlFor="name">Name</Label>
              <Input
                id="name"
                placeholder="customer-support"
                value={formData.name}
                onChange={(e) => handleNameChange(e.target.value)}
                required
              />
              {formData.name.length >= 2 && (
                <div className="flex items-center gap-1.5 text-xs">
                  {nameAvailability.isChecking ? (
                    <>
                      <Loader2 className="w-3 h-3 animate-spin text-muted-foreground" />
                      <span className="text-muted-foreground">
                        Checking availability…
                      </span>
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
                Unique identifier used in URLs and API. Lowercase letters,
                numbers, and hyphens.
              </p>
            </div>

            <div className="space-y-2">
              <Label htmlFor="description">Description</Label>
              <Textarea
                id="description"
                placeholder="Describe what this agent does..."
                value={formData.description}
                onChange={(e) =>
                  setFormData({ ...formData, description: e.target.value })
                }
                rows={2}
              />
            </div>

            <div className="space-y-2">
              <Label htmlFor="model">Model (optional)</Label>
              <ModelPicker
                value={formData.default_model_id}
                onChange={(value) =>
                  setFormData({ ...formData, default_model_id: value })
                }
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
                disabled={createAgent.isPending}
                label=""
              />
              <p className="text-xs text-muted-foreground">
                Add capabilities to give your agent tools and specialized
                behaviors
              </p>
            </div>

            <div className="space-y-2">
              <Label htmlFor="system_prompt">System Prompt</Label>
              <PromptEditor
                id="system_prompt"
                placeholder="You are a helpful assistant..."
                value={formData.system_prompt}
                onChange={(value) =>
                  setFormData({ ...formData, system_prompt: value })
                }
                required
              />
              <p className="text-xs text-muted-foreground">
                Instructions for the AI model (supports Markdown)
              </p>
            </div>

            <InitialFilesEditor
              value={initialFiles}
              onChange={setInitialFiles}
              disabled={createAgent.isPending}
              description="Seed skill files, helper scripts, configs, or binaries into each new session."
            />

            <div className="flex gap-4">
              <Button type="submit" disabled={createAgent.isPending}>
                {createAgent.isPending ? "Creating..." : "Create Agent"}
              </Button>
              <Button
                type="button"
                variant="outline"
                onClick={() => router.back()}
              >
                Cancel
              </Button>
            </div>

            {createAgent.error && (
              <p className="text-sm text-destructive">
                Error: {createAgent.error.message}
              </p>
            )}
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
