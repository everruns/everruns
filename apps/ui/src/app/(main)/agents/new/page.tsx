"use client";

import { useState, useCallback } from "react";
import { useRouter } from "next/navigation";
import { useCreateAgent, useCapabilities, useAgentNameAvailability, usePageTitle } from "@/hooks";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { TagInput } from "@/components/ui/tag-input";
import { PromptEditor } from "@/components/ui/prompt-editor";
import { Boxes, Check, X, Loader2 } from "lucide-react";
import { ModelPicker } from "@/components/models/model-picker";
import { HarnessSelect } from "@/components/harness/harness-select";
import { CapabilitySelector } from "@/components/agents/capability-selector";
import { InitialFilesEditor } from "@/components/initial-files-editor";
import { NetworkAccessEditor } from "@/components/network-access-editor";
import {
  BackLink,
  PageBreadcrumb,
  PageColumns,
  PageContainer,
  PageControlStrip,
  PageFooter,
  PageJumpNav,
  PageMain,
  PageMasthead,
  PageRail,
} from "@/components/layout";
import {
  agentFormSchema,
  getFieldErrors,
  parseTagList,
  type FieldErrors,
} from "@/lib/form-validation";
import type { AgentCapabilityConfig, InitialFile, NetworkAccessList } from "@/lib/api/types";

/** Convert a display name to a slug: lowercase, non-alphanumeric → hyphens, deduplicate, trim. */
function slugify(value: string): string {
  return value
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .replace(/-{2,}/g, "-");
}

export default function NewAgentPage() {
  usePageTitle("New Agent", "Agents");
  const router = useRouter();
  const createAgent = useCreateAgent();
  const { data: allCapabilities = [] } = useCapabilities();

  const [formData, setFormData] = useState({
    display_name: "",
    name: "",
    description: "",
    system_prompt: "",
    harness_id: "",
    default_model_id: "",
    tags: "",
  });

  // Track whether the user has manually edited the slug
  const [nameManuallyEdited, setNameManuallyEdited] = useState(false);

  const nameAvailability = useAgentNameAvailability(formData.name);

  const [selectedCapabilities, setSelectedCapabilities] = useState<AgentCapabilityConfig[]>([]);
  const [initialFiles, setInitialFiles] = useState<InitialFile[]>([]);
  const [networkAccess, setNetworkAccess] = useState<NetworkAccessList>({});
  const [fieldErrors, setFieldErrors] = useState<FieldErrors>({});

  const handleCapabilitiesChange = useCallback((capabilities: AgentCapabilityConfig[]) => {
    setSelectedCapabilities(capabilities);
  }, []);

  const handleDisplayNameChange = (value: string) => {
    const updates: Partial<typeof formData> = { display_name: value };
    // Auto-derive slug unless user has manually edited it
    if (!nameManuallyEdited) {
      updates.name = slugify(value);
    }
    setFormData((prev) => ({ ...prev, ...updates }));
    setFieldErrors((prev) => ({
      ...prev,
      display_name: undefined,
      ...(nameManuallyEdited ? {} : { name: undefined }),
    }));
  };

  const handleNameChange = (value: string) => {
    setNameManuallyEdited(true);
    setFormData((prev) => ({ ...prev, name: value }));
    setFieldErrors((prev) => ({ ...prev, name: undefined }));
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const parsed = agentFormSchema.safeParse(formData);
    if (!parsed.success) {
      setFieldErrors(getFieldErrors(parsed.error));
      return;
    }

    try {
      const tags = parseTagList(parsed.data.tags);
      const agent = await createAgent.mutateAsync({
        name: parsed.data.name,
        display_name: parsed.data.display_name,
        description: parsed.data.description,
        system_prompt: parsed.data.system_prompt,
        harness_id: parsed.data.harness_id,
        default_model_id: parsed.data.default_model_id,
        tags: tags.length > 0 ? tags : undefined,
        capabilities: selectedCapabilities.length > 0 ? selectedCapabilities : undefined,
        initial_files: initialFiles.length > 0 ? initialFiles : undefined,
        network_access:
          networkAccess.allowed?.length || networkAccess.blocked?.length
            ? networkAccess
            : undefined,
      });

      router.push(`/agents/${agent.id}`);
    } catch (error) {
      console.error("Failed to create agent:", error);
    }
  };

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Agents", href: "/agents" }, { label: "New" }]} />

      <PageMasthead
        icon={<Boxes />}
        title="New Agent"
        description="Define the identity, behavior, files, and network policy for new sessions."
        actions={
          <>
            <Button type="submit" form="agent-create-form" disabled={createAgent.isPending}>
              <Check className="size-4" />
              {createAgent.isPending ? "Creating..." : "Create Agent"}
            </Button>
            <Button type="button" variant="outline" onClick={() => router.back()}>
              Cancel
            </Button>
          </>
        }
      />

      <PageControlStrip className="border-b">
        <PageJumpNav
          items={[
            { href: "#identity", label: "Identity" },
            { href: "#behavior", label: "Behavior" },
            { href: "#files", label: "Files" },
            { href: "#network", label: "Network" },
          ]}
          className="px-3.5"
        />
      </PageControlStrip>

      <form id="agent-create-form" onSubmit={handleSubmit}>
        <PageColumns>
          <PageMain>
            <Card id="identity" className="scroll-mt-6">
              <CardHeader>
                <CardTitle>Identity</CardTitle>
                <CardDescription>How the agent is named and found</CardDescription>
              </CardHeader>
              <CardContent className="space-y-6">
                <div className="space-y-2">
                  <Label htmlFor="name">Name</Label>
                  <Input
                    id="name"
                    placeholder="customer-support"
                    value={formData.name}
                    onChange={(e) => handleNameChange(e.target.value)}
                    aria-invalid={!!fieldErrors.name}
                    required
                  />
                  {fieldErrors.name && (
                    <p className="text-xs text-destructive">{fieldErrors.name}</p>
                  )}
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
                  <p className="text-xs text-muted-foreground">
                    Unique identifier used in URLs and API. Lowercase letters, numbers, and hyphens.
                  </p>
                </div>

                <div className="space-y-2">
                  <Label htmlFor="display_name">Display Name</Label>
                  <Input
                    id="display_name"
                    placeholder={formData.name ? undefined : "Customer Support Agent"}
                    value={formData.display_name}
                    onChange={(e) => handleDisplayNameChange(e.target.value)}
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
                    onChange={(e) => setFormData({ ...formData, description: e.target.value })}
                    rows={2}
                  />
                </div>

                <div className="space-y-2">
                  <Label htmlFor="tags">Tags</Label>
                  <TagInput
                    id="tags"
                    placeholder="Add a tag…"
                    value={formData.tags}
                    onChange={(value) => setFormData((prev) => ({ ...prev, tags: value }))}
                    disabled={createAgent.isPending}
                    aria-describedby="tags-help"
                  />
                  <p id="tags-help" className="text-xs text-muted-foreground">
                    Press Enter or comma to add a tag. Backspace removes the last tag.
                  </p>
                </div>
              </CardContent>
            </Card>

            <Card id="behavior" className="scroll-mt-6">
              <CardHeader>
                <CardTitle>Behavior</CardTitle>
                <CardDescription>Execution target and instructions</CardDescription>
              </CardHeader>
              <CardContent className="space-y-6">
                <div className="grid gap-6 md:grid-cols-2">
                  <div className="space-y-2">
                    <Label htmlFor="harness">
                      Harness <span className="text-destructive">*</span>
                    </Label>
                    <HarnessSelect
                      id="harness"
                      value={formData.harness_id}
                      onValueChange={(value) => {
                        setFormData((prev) => ({ ...prev, harness_id: value }));
                        setFieldErrors((prev) => ({ ...prev, harness_id: undefined }));
                      }}
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
                      value={formData.default_model_id}
                      onChange={(value) => setFormData({ ...formData, default_model_id: value })}
                      placeholder="Use default model"
                    />
                    <p className="text-xs text-muted-foreground">
                      Select a specific model or leave empty to use the default
                    </p>
                  </div>
                </div>

                <div className="space-y-2">
                  <Label htmlFor="system_prompt">System Prompt</Label>
                  <PromptEditor
                    id="system_prompt"
                    placeholder="You are a helpful assistant..."
                    value={formData.system_prompt}
                    onChange={(value) => {
                      setFormData({ ...formData, system_prompt: value });
                      setFieldErrors((prev) => ({ ...prev, system_prompt: undefined }));
                    }}
                    required
                  />
                  {fieldErrors.system_prompt && (
                    <p className="text-xs text-destructive">{fieldErrors.system_prompt}</p>
                  )}
                  <p className="text-xs text-muted-foreground">
                    Instructions for the AI model (supports Markdown)
                  </p>
                </div>
              </CardContent>
            </Card>

            <Card id="files" className="scroll-mt-6">
              <CardContent className="pt-6">
                <InitialFilesEditor
                  value={initialFiles}
                  onChange={setInitialFiles}
                  disabled={createAgent.isPending}
                  description="Seed skill files, helper scripts, configs, or binaries into each new session."
                />
              </CardContent>
            </Card>

            <Card id="network" className="scroll-mt-6">
              <CardContent className="pt-6">
                <NetworkAccessEditor
                  value={networkAccess}
                  onChange={setNetworkAccess}
                  disabled={createAgent.isPending}
                  description="Control which hosts this agent's sessions can reach. The selected harness may narrow this policy further."
                />
              </CardContent>
            </Card>
          </PageMain>

          <PageRail>
            <Card>
              <CardHeader>
                <CardTitle>Capabilities</CardTitle>
                <CardDescription>Tools and specialized behaviors</CardDescription>
              </CardHeader>
              <CardContent>
                <CapabilitySelector
                  capabilities={allCapabilities}
                  selected={selectedCapabilities}
                  onChange={handleCapabilitiesChange}
                  disabled={createAgent.isPending}
                />
              </CardContent>
            </Card>

            {createAgent.error && (
              <p className="text-sm text-destructive">Error: {createAgent.error.message}</p>
            )}
          </PageRail>
        </PageColumns>
      </form>

      <PageFooter>
        <BackLink href="/agents">Back to Agents</BackLink>
      </PageFooter>
    </PageContainer>
  );
}
