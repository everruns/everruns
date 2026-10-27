"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { useTextPageFormDraft } from "@/components/chat-mode/use-text-page-form-draft";

import { SandboxTemplateIcon } from "@/components/icons/facet-icons";
import { useState } from "react";
import { useRouter } from "next/navigation";
import { Loader2 } from "lucide-react";
import { SandboxPolicyEditor } from "@/components/agents/sandbox-policy-editor";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { PageBreadcrumb, PageContainer, PageMain, PageMasthead } from "@/components/layout";
import { useCreateSandboxTemplate, usePageTitle } from "@/hooks";
import type { SandboxPolicy } from "@/lib/api/types";

export default function NewSandboxTemplatePage() {
  const ui = useUiTranslations();
  usePageTitle(ui("New Sandbox Template"), ui("Sandbox Templates"));
  const router = useRouter();
  const mutation = useCreateSandboxTemplate();
  const [name, setName] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [description, setDescription] = useState("");
  const [sandboxPolicy, setSandboxPolicy] = useState<SandboxPolicy | null>(null);
  const spec = sandboxPolicy?.templates?.[sandboxPolicy.default];
  const chatDraft = useTextPageFormDraft({
    fields: ["name", "display_name", "description"],
    values: { name, display_name: displayName, description },
    entityType: "sandbox_template",
    label: ui("New Sandbox Template"),
    baseline: "new",
    dirty: !!(name || displayName || description),
    writable: !mutation.isPending,
    apply: (values) => {
      setName(values.name);
      setDisplayName(values.display_name);
      setDescription(values.description);
    },
  });
  const submit = async () => {
    if (chatDraft.blocked) return;
    if (!name || !displayName || !spec) return;
    const created = await mutation.mutateAsync({
      name,
      display_name: displayName,
      ...(description ? { description } : {}),
      spec,
    });
    router.push(`/sandboxes/templates/${created.id}`);
  };
  return (
    <PageContainer>
      <PageBreadcrumb
        items={[
          { label: ui("Sandboxes"), href: "/sandboxes" },
          { label: ui("Templates"), href: "/sandboxes/templates" },
          { label: ui("New") },
        ]}
      />
      <PageMasthead
        icon={<SandboxTemplateIcon />}
        title={ui("New Sandbox Template")}
        description={ui(
          "Create reusable Sandbox configuration. Saving later creates immutable revisions.",
        )}
      />
      <PageMain className="max-w-3xl space-y-5">
        {chatDraft.notice}
        <Card>
          <CardHeader>
            <CardTitle>{ui("Identity")}</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="name">{ui("Name")}</Label>
              <Input
                id="name"
                value={name}
                onChange={(event) => setName(event.target.value.toLowerCase())}
                placeholder={ui("coding-daytona")}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="display-name">{ui("Display name")}</Label>
              <Input
                id="display-name"
                value={displayName}
                onChange={(event) => setDisplayName(event.target.value)}
                placeholder={ui("Coding Daytona")}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="description">{ui("Description")}</Label>
              <Textarea
                id="description"
                value={description}
                onChange={(event) => setDescription(event.target.value)}
              />
            </div>
          </CardContent>
        </Card>
        <Card>
          <CardHeader>
            <CardTitle>{ui("Sandbox configuration")}</CardTitle>
          </CardHeader>
          <CardContent>
            <SandboxPolicyEditor value={sandboxPolicy} onChange={setSandboxPolicy} definitionMode />
          </CardContent>
        </Card>
        <Button
          variant="accent"
          onClick={submit}
          disabled={!name || !displayName || !spec || mutation.isPending}
        >
          {mutation.isPending ? <Loader2 className="size-4 animate-spin" /> : null}
          {ui("Create Sandbox Template")}
        </Button>
        {mutation.error ? (
          <p className="text-sm text-destructive">{mutation.error.message}</p>
        ) : null}
      </PageMain>
    </PageContainer>
  );
}
