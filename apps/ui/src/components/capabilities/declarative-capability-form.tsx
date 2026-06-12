"use client";

import { useMemo, useState, type FormEvent } from "react";
import { useRouter } from "next/navigation";
import Link from "next/link";
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
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { ArrowLeft } from "lucide-react";
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

function TabCount({ count }: { count: number }) {
  if (count === 0) return null;
  return (
    <Badge variant="secondary" className="ml-1.5 h-4 px-1.5 text-[10px]">
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

  return (
    <div className="container mx-auto p-6">
      <Link
        href="/capabilities"
        className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-6"
      >
        <ArrowLeft className="w-4 h-4 mr-2" />
        Back to Capabilities
      </Link>

      <form onSubmit={handleSubmit} className="space-y-6">
        <div className="flex items-start justify-between gap-4 flex-wrap">
          <h1 className="text-2xl font-bold">
            {mode === "create" ? "New Declarative Capability" : "Edit Declarative Capability"}
          </h1>
          <div className="flex items-center gap-2">
            <Button type="button" variant="outline" onClick={() => router.push("/capabilities")}>
              Cancel
            </Button>
            <Button type="submit" disabled={isPending || !canSubmit}>
              {isPending ? "Saving..." : mode === "create" ? "Create" : "Save changes"}
            </Button>
          </div>
        </div>

        <Tabs value={tab} onValueChange={setTab}>
          <TabsList>
            <TabsTrigger value="general">General</TabsTrigger>
            <TabsTrigger value="mcp">
              MCP Servers
              <TabCount count={state.mcpServers.length} />
            </TabsTrigger>
            <TabsTrigger value="skills">
              Skills
              <TabCount count={state.skills.length} />
            </TabsTrigger>
            <TabsTrigger value="files">
              Files
              <TabCount count={state.files.length} />
            </TabsTrigger>
          </TabsList>

          <TabsContent value="general">
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
          </TabsContent>

          <TabsContent value="mcp">
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
          </TabsContent>

          <TabsContent value="skills">
            <Card>
              <CardHeader>
                <CardTitle className="text-base">Skills</CardTitle>
              </CardHeader>
              <CardContent>
                <SkillsEditor skills={state.skills} onChange={(skills) => set("skills", skills)} />
              </CardContent>
            </Card>
          </TabsContent>

          <TabsContent value="files">
            <Card>
              <CardHeader>
                <CardTitle className="text-base">Files</CardTitle>
              </CardHeader>
              <CardContent>
                <FilesEditor files={state.files} onChange={(files) => set("files", files)} />
              </CardContent>
            </Card>
          </TabsContent>
        </Tabs>

        {error && <p className="text-sm text-destructive">{error}</p>}
      </form>
    </div>
  );
}
