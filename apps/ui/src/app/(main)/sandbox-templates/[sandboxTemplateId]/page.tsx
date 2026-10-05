"use client";

import { useState } from "react";
import { useParams, useRouter } from "next/navigation";
import { Archive, Container, Loader2 } from "lucide-react";
import { SandboxPolicyEditor } from "@/components/agents/sandbox-policy-editor";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { PageBreadcrumb, PageContainer, PageMain, PageMasthead } from "@/components/layout";
import {
  useArchiveSandboxTemplate,
  usePageTitle,
  useReviseSandboxTemplate,
  useSandboxTemplate,
} from "@/hooks";
import type { SandboxPolicy, SandboxTemplate } from "@/lib/api/types";

export default function SandboxTemplatePage() {
  const { sandboxTemplateId } = useParams<{ sandboxTemplateId: string }>();
  const { data: template, isLoading } = useSandboxTemplate(sandboxTemplateId);
  usePageTitle(template?.display_name || "Sandbox Template", "Sandbox Templates");
  if (isLoading || !template) return null;
  return (
    <SandboxTemplateEditor
      key={`${template.id}:${template.current_revision.id}`}
      template={template}
    />
  );
}

function SandboxTemplateEditor({ template }: { template: SandboxTemplate }) {
  const router = useRouter();
  const revise = useReviseSandboxTemplate();
  const archive = useArchiveSandboxTemplate();
  const [displayName, setDisplayName] = useState(template.display_name);
  const [description, setDescription] = useState(template.description || "");
  const [configuration, setConfiguration] = useState<SandboxPolicy | null>(() => ({
    mode: "fixed",
    default: template.name,
    templates: { [template.name]: template.current_revision.spec },
  }));
  const spec = configuration?.templates?.[configuration.default];
  const save = async () => {
    if (!spec) return;
    await revise.mutateAsync({
      id: template.id,
      display_name: displayName,
      description: description || null,
      spec,
    });
  };
  return (
    <PageContainer>
      <PageBreadcrumb
        items={[
          { label: "Sandbox Templates", href: "/sandbox-templates" },
          { label: template.display_name },
        ]}
      />
      <PageMasthead
        icon={<Container />}
        title={template.display_name}
        description={`${template.name} · revision ${template.current_revision.revision}${template.is_managed ? " · managed" : ""}`}
      />
      <PageMain className="max-w-3xl space-y-5">
        <Card>
          <CardHeader>
            <CardTitle>Configuration</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="display-name">Display name</Label>
              <Input
                id="display-name"
                value={displayName}
                onChange={(event) => setDisplayName(event.target.value)}
                disabled={template.is_managed}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="description">Description</Label>
              <Textarea
                id="description"
                value={description}
                onChange={(event) => setDescription(event.target.value)}
                disabled={template.is_managed}
              />
            </div>
            <SandboxPolicyEditor
              value={configuration}
              onChange={setConfiguration}
              disabled={template.is_managed}
              definitionMode
            />
          </CardContent>
        </Card>
        {!template.is_managed ? (
          <div className="flex gap-2">
            <Button variant="accent" onClick={save} disabled={!spec || revise.isPending}>
              {revise.isPending ? <Loader2 className="size-4 animate-spin" /> : null}Save new
              revision
            </Button>
            <Button
              variant="outline"
              onClick={async () => {
                await archive.mutateAsync(template.id);
                router.push("/sandbox-templates");
              }}
            >
              <Archive className="size-4" />
              Archive
            </Button>
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">
            Managed Sandbox Templates are versioned by the platform and cannot be edited.
          </p>
        )}
      </PageMain>
    </PageContainer>
  );
}
