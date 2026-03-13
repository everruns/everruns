"use client";

import { use, useMemo, useCallback, useState } from "react";
import {
  useHarness,
  useCapabilities,
  useLlmModels,
  useDeleteHarness,
  useCopyHarness,
} from "@/hooks";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { MarkdownDisplay } from "@/components/ui/prompt-editor";
import { InlineStreamdownMessage } from "@/components/chat/streamdown-message";
import { ProviderIcon } from "@/components/providers/provider-icon";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import { HarnessPreview } from "@/components/harnesses/harness-preview";
import { ArrowLeft, Pencil, Copy, Trash2, Eye, LayoutDashboard } from "lucide-react";
import { CopyButton } from "@/components/ui/copy-button";
import type { Capability, LlmModelWithProvider } from "@/lib/api/types";
import { getCapabilityIcon } from "@/lib/capability-icons";

export default function HarnessDetailPage({ params }: { params: Promise<{ harnessId: string }> }) {
  const { harnessId } = use(params);
  const router = useRouter();
  const [activeTab, setActiveTab] = useState("overview");
  const { data: harness, isLoading: harnessLoading } = useHarness(harnessId);
  const { data: allCapabilities } = useCapabilities();
  const { data: llmModels } = useLlmModels();
  const deleteHarness = useDeleteHarness();
  const copyHarness = useCopyHarness();

  const modelMap = useMemo(() => {
    if (!llmModels) return new Map<string, LlmModelWithProvider>();
    return new Map(llmModels.map((m) => [m.id, m]));
  }, [llmModels]);

  const defaultModel = harness?.default_model_id
    ? modelMap.get(harness.default_model_id)
    : undefined;

  const getCapabilityInfo = (capabilityId: string): Capability | undefined =>
    allCapabilities?.find((c) => c.id === capabilityId);

  const harnessCapabilities = harness?.capabilities ?? [];

  const handleDelete = async () => {
    if (!confirm("Are you sure you want to delete this harness? This action cannot be undone.")) {
      return;
    }
    try {
      await deleteHarness.mutateAsync(harnessId);
      router.push("/harnesses");
    } catch (error) {
      console.error("Failed to delete harness:", error);
    }
  };

  const handleCopy = useCallback(async () => {
    try {
      const copied = await copyHarness.mutateAsync(harnessId);
      router.push(`/harnesses/${copied.id}`);
    } catch (error) {
      console.error("Failed to copy harness:", error);
    }
  }, [harnessId, copyHarness, router]);

  if (harnessLoading) {
    return (
      <div className="container mx-auto p-6">
        <Skeleton className="h-8 w-1/3 mb-4" />
        <Skeleton className="h-4 w-2/3 mb-8" />
        <Skeleton className="h-64 w-full" />
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
        href="/harnesses"
        className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-6"
      >
        <ArrowLeft className="w-4 h-4 mr-2" />
        Back to Harnesses
      </Link>

      <div className="flex items-start justify-between mb-6">
        <div>
          <h1 className="text-2xl font-bold flex items-center gap-2">
            {harness.name}
            <CopyButton value={harness.id} />
            {harness.is_built_in && (
              <Badge variant="outline">Built-in</Badge>
            )}
            <Badge variant={harness.status === "active" ? "default" : "secondary"}>
              {harness.status}
            </Badge>
          </h1>
        </div>
        <div className="flex gap-2">
          <Button variant="outline" onClick={handleCopy} disabled={copyHarness.isPending}>
            <Copy className="w-4 h-4 mr-2" />
            {copyHarness.isPending ? "Copying..." : "Copy"}
          </Button>
          {!harness.is_built_in && (
            <>
              <Link href={`/harnesses/${harnessId}/edit`}>
                <Button variant="outline">
                  <Pencil className="w-4 h-4 mr-2" />
                  Edit
                </Button>
              </Link>
              <Button variant="destructive" onClick={handleDelete} disabled={deleteHarness.isPending}>
                <Trash2 className="w-4 h-4 mr-2" />
                {deleteHarness.isPending ? "Deleting..." : "Delete"}
              </Button>
            </>
          )}
        </div>
      </div>

      <Tabs value={activeTab} onValueChange={setActiveTab}>
        <TabsList className="mb-6">
          <TabsTrigger value="overview">
            <LayoutDashboard className="w-4 h-4 mr-2" />
            Overview
          </TabsTrigger>
          <TabsTrigger value="preview">
            <Eye className="w-4 h-4 mr-2" />
            Preview
          </TabsTrigger>
        </TabsList>

        <TabsContent value="overview">
          <div className="grid gap-6 lg:grid-cols-3">
            <div className="lg:col-span-2 space-y-6">
              <Card>
                <CardHeader>
                  <CardTitle>System Prompt</CardTitle>
                </CardHeader>
                <CardContent>
                  <MarkdownDisplay content={harness.system_prompt} />
                </CardContent>
              </Card>
            </div>

            <div className="space-y-6">
              <Card>
                <CardHeader>
                  <CardTitle>Capabilities</CardTitle>
                </CardHeader>
                <CardContent>
                  {harnessCapabilities.length === 0 ? (
                    <p className="text-sm text-muted-foreground">
                      No capabilities enabled.{" "}
                      <Link
                        href={`/harnesses/${harnessId}/edit`}
                        className="text-primary hover:underline"
                      >
                        Add some
                      </Link>
                    </p>
                  ) : (
                    <div className="space-y-2">
                      {harnessCapabilities.map((capConfig) => {
                        const cap = getCapabilityInfo(capConfig.ref);
                        if (!cap) return null;
                        const IconComponent = getCapabilityIcon(cap.icon);

                        return (
                          <div
                            key={capConfig.ref}
                            className="flex items-center gap-2 p-2 rounded-md border bg-muted/50"
                          >
                            <IconComponent className="w-4 h-4" />
                            <div className="flex-1">
                              <p className="text-sm font-medium">{cap.name}</p>
                              <p className="text-xs text-muted-foreground">{cap.description}</p>
                            </div>
                          </div>
                        );
                      })}
                    </div>
                  )}
                </CardContent>
              </Card>

              <Card>
                <CardHeader>
                  <CardTitle>Configuration</CardTitle>
                </CardHeader>
                <CardContent className="space-y-4">
                  {defaultModel && (
                    <div>
                      <p className="text-sm font-medium mb-2">Default Model</p>
                      <div className="flex items-center gap-2">
                        <ProviderIcon providerType={defaultModel.provider_type} size="sm" />
                        <span className="text-sm">{defaultModel.display_name}</span>
                      </div>
                    </div>
                  )}

                  {harness.description && (
                    <div>
                      <p className="text-sm font-medium">Description</p>
                      <div className="text-sm text-muted-foreground">
                        <InlineStreamdownMessage>{harness.description}</InlineStreamdownMessage>
                      </div>
                    </div>
                  )}

                  {harness.tags.length > 0 && (
                    <div>
                      <p className="text-sm font-medium mb-2">Tags</p>
                      <div className="flex flex-wrap gap-1">
                        {harness.tags.map((tag) => (
                          <Badge key={tag} variant="outline">
                            {tag}
                          </Badge>
                        ))}
                      </div>
                    </div>
                  )}

                  <div>
                    <p className="text-sm font-medium">Created</p>
                    <p className="text-sm text-muted-foreground">
                      {new Date(harness.created_at).toLocaleString()}
                    </p>
                  </div>

                  <div>
                    <p className="text-sm font-medium">Updated</p>
                    <p className="text-sm text-muted-foreground">
                      {new Date(harness.updated_at).toLocaleString()}
                    </p>
                  </div>
                </CardContent>
              </Card>
            </div>
          </div>
        </TabsContent>

        <TabsContent value="preview">
          <HarnessPreview
            systemPrompt={harness.system_prompt}
            capabilities={harnessCapabilities.map((cap) => ({
              ref: cap.ref,
              config: cap.config,
            }))}
          />
        </TabsContent>
      </Tabs>
    </div>
  );
}
