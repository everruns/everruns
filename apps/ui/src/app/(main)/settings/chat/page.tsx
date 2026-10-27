"use client";

import { useState } from "react";
import { z } from "zod";
import { usePageSettingsDraft } from "@/components/chat-mode/use-page-settings-draft";
import { ChatIcon } from "@/components/icons/facet-icons";
import { ModelPicker } from "@/components/models/model-picker";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { usePageTitle } from "@/hooks";
import { useOrganization, useUpdateOrganization } from "@/hooks/use-organizations";
import { useOrg } from "@/providers/org-provider";
import type { Organization } from "@/lib/api/types";

const chatSettingsSchema = z.strictObject({ chat_model_id: z.string() });

export default function ChatSettingsPage() {
  usePageTitle("Chat", "Settings");
  const { hasRole } = useOrg();
  const { data: organization, isLoading, error } = useOrganization();
  if (isLoading) return <Skeleton className="h-48 w-full" />;
  if (error || !organization) return <p role="alert">Unable to load Chat settings.</p>;
  // A new organization gets a fresh draft; pending edits never cross tenants.
  return (
    <ChatModelSettings
      key={organization.id}
      organization={organization}
      canManage={hasRole("admin")}
    />
  );
}

function ChatModelSettings({
  organization,
  canManage,
}: {
  organization: Organization;
  canManage: boolean;
}) {
  const [modelId, setModelId] = useState(organization.chat_model_id || "");
  const [savedId, setSavedId] = useState(organization.chat_model_id || "");
  const [error, setError] = useState<string | null>(null);
  const update = useUpdateOrganization();
  const draft = usePageSettingsDraft({
    values: { chat_model_id: organization.chat_model_id ?? "" },
    schema: chatSettingsSchema,
    entityType: "organization",
    entityId: organization.id,
    formId: "chat_model",
    label: "Chat settings",
    writable: canManage && !update.isPending,
    save: (values) => update.mutateAsync({ chat_model_id: values.chat_model_id || null }),
  });
  const selectedModel = draft.adopted ? draft.values.chat_model_id : modelId;
  async function save() {
    if (!canManage || update.isPending) return;
    setError(null);
    try {
      const saved = await update.mutateAsync({ chat_model_id: modelId || null });
      setSavedId(saved.chat_model_id || "");
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Unable to save Chat settings.");
    }
  }
  return (
    <div className="space-y-6">
      <div className="flex items-center gap-3">
        <ChatIcon className="size-5" />
        <h1 className="text-xl font-semibold">Chat settings</h1>
      </div>
      <Card className="space-y-4 p-6">
        <div className="space-y-1">
          <h2 className="font-medium">Default Chat model</h2>
          <p className="text-sm text-muted-foreground">
            Choose the model for Everruns Chat in this organization. Agents, sessions, and their
            Work Chat channels keep their own model settings. Explicit conversation selections take
            priority.
          </p>
        </div>
        <div className="max-w-lg space-y-2">
          <Label htmlFor="chat-model">Model</Label>
          <ModelPicker
            id="chat-model"
            value={selectedModel}
            onChange={(id) =>
              draft.adopted ? draft.setValues({ chat_model_id: id }) : setModelId(id)
            }
            service="chat"
            organizationDefault
            requireTools
            disabled={!canManage || update.isPending}
            placeholder="Use agents and sessions default"
          />
        </div>
        {draft.controls}
        {!canManage && (
          <p className="text-sm text-muted-foreground">
            Only organization owners and admins can change this setting.
          </p>
        )}
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
        {canManage && !draft.adopted && (
          <Button onClick={save} disabled={update.isPending || modelId === savedId}>
            {update.isPending ? "Saving…" : "Save"}
          </Button>
        )}
      </Card>
    </div>
  );
}
