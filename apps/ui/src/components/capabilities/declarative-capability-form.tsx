"use client";

import { useMemo, useState, type FormEvent } from "react";
import { useRouter } from "next/navigation";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Badge } from "@/components/ui/badge";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Pencil, Plus, Settings, Wrench, BookOpen, FileText } from "lucide-react";
import { registryDomainIcons } from "@/lib/registry-navigation";
import type { DeclarativeCapabilityDefinition } from "@/lib/api/types";
import { useCreateDeclarativeCapability, useUpdateDeclarativeCapability } from "@/hooks";
import { McpServersEditor } from "./mcp-servers-editor";
import { SkillsEditor } from "./skills-editor";
import { FilesEditor } from "./files-editor";
import {
  definitionToFormState,
  formStateToDefinition,
  type DeclarativeFormState,
} from "./declarative-form-types";
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
  type SectionTabItem,
} from "@/components/layout";

const FORM_ID = "capability-edit-form";
const CapabilitiesIcon = registryDomainIcons.capabilities;

function TabCount({ count }: { count: number }) {
  if (count === 0) return null;
  return (
    <Badge variant="secondary" className="ml-0.5 h-4 px-1.5 text-[10px]">
      {count}
    </Badge>
  );
}

export function DeclarativeCapabilityForm({
  mode,
  id,
  initialDefinition,
}: {
  mode: "create" | "edit";
  id?: string;
  initialDefinition?: DeclarativeCapabilityDefinition;
}) {
  const router = useRouter();
  const createCapability = useCreateDeclarativeCapability();
  const updateCapability = useUpdateDeclarativeCapability();

  const [state, setState] = useState<DeclarativeFormState>(() =>
    definitionToFormState(initialDefinition),
  );
  const [tab, setTab] = useState("general");
  const [error, setError] = useState<string | null>(null);

  const set = <K extends keyof DeclarativeFormState>(key: K, value: DeclarativeFormState[K]) =>
    setState((prev) => ({ ...prev, [key]: value }));

  const isPending = createCapability.isPending || updateCapability.isPending;
  const canSubmit = state.name.trim().length > 0 && state.description.trim().length > 0;

  const definition = useMemo(() => formStateToDefinition(state), [state]);

  const handleSubmit = async (event: FormEvent) => {
    event.preventDefault();
    setError(null);
    try {
      if (mode === "create") {
        await createCapability.mutateAsync({ definition });
      } else if (id) {
        await updateCapability.mutateAsync({ id, request: { definition } });
      }
      router.push("/capabilities");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to save capability.");
    }
  };

  const tabItems: SectionTabItem[] = [
    { value: "general", label: "General", icon: <Settings className="size-4" /> },
    {
      value: "mcp",
      label: (
        <>
          MCP Servers
          <TabCount count={state.mcpServers.length} />
        </>
      ),
      icon: <Wrench className="size-4" />,
    },
    {
      value: "skills",
      label: (
        <>
          Skills
          <TabCount count={state.skills.length} />
        </>
      ),
      icon: <BookOpen className="size-4" />,
    },
    {
      value: "files",
      label: (
        <>
          Files
          <TabCount count={state.files.length} />
        </>
      ),
      icon: <FileText className="size-4" />,
    },
  ];

  const title =
    mode === "create" ? "New capability" : state.displayName || state.name || "Capability";

  return (
    <PageContainer>
      <PageBreadcrumb
        items={[
          { label: "Capabilities", href: "/capabilities" },
          { label: mode === "create" ? "New Declarative" : "Edit" },
        ]}
      />

      <PageMasthead
        icon={<CapabilitiesIcon />}
        title={title}
        badges={
          <Badge variant="accent">
            {mode === "create" ? (
              <>
                <Plus className="size-3" />
                New
              </>
            ) : (
              <>
                <Pencil className="size-3" />
                Editing
              </>
            )}
          </Badge>
        }
        description="Declarative capabilities bundle MCP servers, skills, and files into a reusable building block."
        meta={
          mode === "edit" ? (
            <span>
              Referenced as <span className="font-mono text-primary">declarative:{state.name}</span>
            </span>
          ) : undefined
        }
        actions={
          <>
            <Button type="submit" form={FORM_ID} disabled={isPending || !canSubmit}>
              <Pencil className="size-4" />
              {isPending ? "Saving..." : mode === "create" ? "Create" : "Save changes"}
            </Button>
            <Button type="button" variant="outline" onClick={() => router.back()}>
              Discard
            </Button>
          </>
        }
      />

      <PageControlStrip>
        <SectionTabs value={tab} onValueChange={setTab} items={tabItems} />
      </PageControlStrip>

      <form id={FORM_ID} onSubmit={handleSubmit}>
        <PageColumns>
          <PageMain>
            {tab === "general" && (
              <Card>
                <CardContent className="space-y-4 pt-6">
                  <div className="grid gap-2">
                    <Label htmlFor="declarative-name">Unique name</Label>
                    <Input
                      id="declarative-name"
                      placeholder="research_pack"
                      value={state.name}
                      onChange={(e) => set("name", e.target.value.toLowerCase())}
                      disabled={mode === "edit"}
                      required
                    />
                    <p className="text-xs text-muted-foreground">
                      Lowercase letters, digits, <code>_</code> and <code>-</code>. Referenced as{" "}
                      <code>declarative:{state.name || "name"}</code>.
                      {mode === "edit" && " The name cannot be changed after creation."}
                    </p>
                  </div>
                  <div className="grid gap-2">
                    <Label htmlFor="declarative-display-name">Display name</Label>
                    <Input
                      id="declarative-display-name"
                      placeholder="Research Pack"
                      value={state.displayName}
                      onChange={(e) => set("displayName", e.target.value)}
                    />
                  </div>
                  <div className="grid gap-2">
                    <Label htmlFor="declarative-description">Description</Label>
                    <Input
                      id="declarative-description"
                      value={state.description}
                      onChange={(e) => set("description", e.target.value)}
                      required
                    />
                  </div>
                  <div className="grid gap-2">
                    <Label htmlFor="declarative-prompt">System prompt addition</Label>
                    <Textarea
                      id="declarative-prompt"
                      value={state.systemPrompt}
                      onChange={(e) => set("systemPrompt", e.target.value)}
                      rows={8}
                    />
                  </div>
                  <div className="grid gap-2">
                    <Label htmlFor="declarative-risk">Risk level</Label>
                    <Select
                      value={state.riskLevel}
                      onValueChange={(value) =>
                        set("riskLevel", value as DeclarativeFormState["riskLevel"])
                      }
                    >
                      <SelectTrigger id="declarative-risk" className="w-48">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="low">Low</SelectItem>
                        <SelectItem value="medium">Medium</SelectItem>
                        <SelectItem value="high">High (admin only)</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                </CardContent>
              </Card>
            )}

            {tab === "mcp" && (
              <Card>
                <CardHeader>
                  <CardTitle className="text-base">MCP Servers</CardTitle>
                </CardHeader>
                <CardContent>
                  <McpServersEditor
                    servers={state.mcpServers}
                    onChange={(servers) => set("mcpServers", servers)}
                  />
                </CardContent>
              </Card>
            )}

            {tab === "skills" && (
              <Card>
                <CardHeader>
                  <CardTitle className="text-base">Skills</CardTitle>
                </CardHeader>
                <CardContent>
                  <SkillsEditor
                    skills={state.skills}
                    onChange={(skills) => set("skills", skills)}
                  />
                </CardContent>
              </Card>
            )}

            {tab === "files" && (
              <Card>
                <CardHeader>
                  <CardTitle className="text-base">Files</CardTitle>
                </CardHeader>
                <CardContent>
                  <FilesEditor files={state.files} onChange={(files) => set("files", files)} />
                </CardContent>
              </Card>
            )}

            {error && <p className="text-sm text-destructive">{error}</p>}
          </PageMain>

          <PageRail>
            <Card>
              <CardHeader>
                <CardTitle className="text-base">Summary</CardTitle>
              </CardHeader>
              <CardContent className="space-y-3 text-sm">
                <div>
                  <p className="font-medium">Name</p>
                  <p className="text-muted-foreground font-mono">{state.name || "(not set)"}</p>
                </div>
                <div>
                  <p className="font-medium">Display name</p>
                  <p className="text-muted-foreground">{state.displayName || "(not set)"}</p>
                </div>
                <div className="flex items-center justify-between">
                  <span className="font-medium">MCP servers</span>
                  <span className="text-muted-foreground">{state.mcpServers.length}</span>
                </div>
                <div className="flex items-center justify-between">
                  <span className="font-medium">Skills</span>
                  <span className="text-muted-foreground">{state.skills.length}</span>
                </div>
                <div className="flex items-center justify-between">
                  <span className="font-medium">Files</span>
                  <span className="text-muted-foreground">{state.files.length}</span>
                </div>
              </CardContent>
            </Card>
          </PageRail>
        </PageColumns>
      </form>

      <PageFooter>
        <BackLink href="/capabilities">Back to Capabilities</BackLink>
      </PageFooter>
    </PageContainer>
  );
}
