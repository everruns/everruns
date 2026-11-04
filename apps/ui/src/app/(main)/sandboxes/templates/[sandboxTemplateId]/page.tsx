"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { usePageChatMode } from "@/components/chat-mode/chat-mode-context";

import { useTextPageFormDraft } from "@/components/chat-mode/use-text-page-form-draft";

import { SandboxTemplateIcon } from "@/components/icons/facet-icons";
import { useState } from "react";
import { useParams, useRouter } from "next/navigation";
import { Archive, Loader2 } from "lucide-react";
import { SandboxPolicyEditor } from "@/components/agents/sandbox-policy-editor";
import { SandboxTemplateBuildPanel } from "@/components/sandboxes/sandbox-template-build-panel";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
  PageBreadcrumb,
  PageContainer,
  PageMain,
  PageMasthead,
  SectionTabs,
} from "@/components/layout";
import {
  useArchiveSandboxTemplate,
  usePageTitle,
  useReviseSandboxTemplate,
  useSandboxTemplate,
} from "@/hooks";
import type { SandboxPolicy, SandboxTemplate } from "@/lib/api/types";

export default function SandboxTemplatePage() {
  const ui = useUiTranslations();
  const chatMode = usePageChatMode();
  const { sandboxTemplateId } = useParams<{ sandboxTemplateId: string }>();
  const { data: template, isLoading } = useSandboxTemplate(sandboxTemplateId);
  usePageTitle(template?.display_name || "Sandbox Template", ui("Sandbox Templates"));
  if (isLoading || !template) return null;
  return (
    <SandboxTemplateEditor
      key={chatMode ? template.id : `${template.id}:${template.current_revision.id}`}
      template={template}
    />
  );
}

function SandboxTemplateEditor({ template }: { template: SandboxTemplate }) {
  const ui = useUiTranslations();
  const router = useRouter();
  const revise = useReviseSandboxTemplate();
  const archive = useArchiveSandboxTemplate();
  const [displayName, setDisplayName] = useState(template.display_name);
  const [description, setDescription] = useState(template.description || "");
  const [tab, setTab] = useState("configuration");
  const [configuration, setConfiguration] = useState<SandboxPolicy | null>(() => ({
    mode: "fixed",
    default: template.name,
    templates: { [template.name]: template.current_revision.spec },
  }));
  const spec = configuration?.templates?.[configuration.default];
  const dirty =
    displayName !== template.display_name ||
    description !== (template.description ?? "") ||
    JSON.stringify(spec) !== JSON.stringify(template.current_revision.spec);
  const chatDraft = useTextPageFormDraft({
    fields: ["display_name", "description"],
    values: { display_name: displayName, description },
    entityId: template.id,
    entityType: "sandbox_template",
    label: template.display_name,
    baseline: template.current_revision.id,
    dirty:
      displayName !== template.display_name ||
      description !== (template.description ?? "") ||
      JSON.stringify(spec) !== JSON.stringify(template.current_revision.spec),
    writable: !template.is_managed && !revise.isPending,
    // Sandbox environment values remain in the manual configuration editor.
    apply: (values) => {
      setDisplayName(values.display_name);
      setDescription(values.description);
    },
  });
  const save = async () => {
    if (chatDraft.blocked) return;
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
          { label: ui("Sandboxes"), href: "/sandboxes" },
          { label: ui("Templates"), href: "/sandboxes/templates" },
          { label: template.display_name },
        ]}
      />
      <PageMasthead
        icon={<SandboxTemplateIcon />}
        title={template.display_name}
        description={ui("{value0} · revision {value1}{value2}", {
          value0: template.name,
          value1: template.current_revision.revision,
          value2: template.is_managed ? " · managed" : "",
        })}
      />
      {!template.is_managed ? (
        <SectionTabs
          value={tab}
          onValueChange={setTab}
          aria-label={ui("Sandbox Template sections")}
          items={[
            { value: "configuration", label: ui("Configuration") },
            { value: "build", label: ui("Build snapshot") },
          ]}
        />
      ) : null}
      <PageMain className="max-w-3xl space-y-5">
        {!template.is_managed ? (
          <div hidden={tab !== "build"}>
            <SandboxTemplateBuildPanel template={template} dirty={dirty} />
          </div>
        ) : null}
        <div hidden={tab !== "configuration"} className="space-y-5">
          {chatDraft.notice}
          <Card>
            <CardHeader>
              <CardTitle>{ui("Configuration")}</CardTitle>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="space-y-2">
                <Label htmlFor="display-name">{ui("Display name")}</Label>
                <Input
                  id="display-name"
                  value={displayName}
                  onChange={(event) => setDisplayName(event.target.value)}
                  disabled={template.is_managed}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="description">{ui("Description")}</Label>
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
                {revise.isPending ? <Loader2 className="size-4 animate-spin" /> : null}
                {ui("Save new revision")}
              </Button>
              {chatDraft.blocked && (
                <Button
                  variant="outline"
                  onClick={() => {
                    setDisplayName(template.display_name);
                    setDescription(template.description ?? "");
                    setConfiguration({
                      mode: "fixed",
                      default: template.name,
                      templates: { [template.name]: template.current_revision.spec },
                    });
                  }}
                >
                  {ui("Discard")}
                </Button>
              )}
              <Button
                variant="outline"
                onClick={async () => {
                  await archive.mutateAsync(template.id);
                  router.push("/sandboxes/templates");
                }}
              >
                <Archive className="size-4" />
                {ui("Archive")}
              </Button>
            </div>
          ) : (
            <p className="text-sm text-muted-foreground">
              {ui("Managed Sandbox Templates are versioned by the platform and cannot be edited.")}
            </p>
          )}
        </div>
      </PageMain>
    </PageContainer>
  );
}
