"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { usePageFormDraft } from "@/components/chat-mode/use-page-form-draft";
import { evalCreateDraftSchema } from "@/components/chat-mode/page-form-schemas";

import { TagInput } from "@/components/ui/tag-input";
import { Suspense, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { useCreateEval, usePageTitle } from "@/hooks";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import Link from "next/link";
import { ArrowLeft } from "lucide-react";
import { ModelPicker } from "@/components/models/model-picker";
import { AgentSelect } from "@/components/agent/agent-select";
import { HarnessSelect } from "@/components/harness/harness-select";
import type { EvalTarget, CreateEvalRequest } from "@/lib/api/types";

function NewEvalForm() {
  const ui = useUiTranslations();
  usePageTitle(ui("New Eval"), ui("Evals"));
  const router = useRouter();
  const createEval = useCreateEval();
  const [formData, setFormData] = useState({
    name: "",
    description: "",
    model_override: "",
  });
  const [harnessId, setHarnessId] = useState("");
  // `?agent=` preselects the agent, from an agent's Evals tab.
  const searchParams = useSearchParams();
  const [agentId, setAgentId] = useState(searchParams.get("agent") ?? "");
  const [systemPrompt, setSystemPrompt] = useState("");
  const [modelId, setModelId] = useState("");
  const [tags, setTags] = useState<string[]>([]);

  const chatDraft = usePageFormDraft({
    schema: evalCreateDraftSchema,
    values: {
      ...formData,
      harness_id: harnessId,
      agent_id: agentId,
      system_prompt: systemPrompt,
      model_id: modelId,
      tags,
    },
    entityType: "eval",
    label: ui("New Eval"),
    baseline: "new",
    dirty: !!(
      Object.values(formData).some(Boolean) ||
      harnessId ||
      agentId ||
      systemPrompt ||
      modelId ||
      tags.length
    ),
    writable: !createEval.isPending,
    apply: ({ harness_id, agent_id, system_prompt, model_id, tags: nextTags, ...fields }) => {
      setFormData(fields);
      setHarnessId(harness_id);
      setAgentId(agent_id);
      setSystemPrompt(system_prompt);
      setModelId(model_id);
      setTags(nextTags);
    },
  });

  const buildTarget = (): EvalTarget | undefined => {
    return {
      type: "session",
      harness_id: harnessId || undefined,
      agent_id: agentId || undefined,
      model_id: modelId || undefined,
      system_prompt: systemPrompt || undefined,
    };
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (chatDraft.blocked) return;

    const target = buildTarget();

    const req: CreateEvalRequest = {
      name: formData.name,
      description: formData.description || undefined,
      target,
      model_override: formData.model_override || undefined,
      tags: tags.length > 0 ? tags : undefined,
    };

    try {
      const ev = await createEval.mutateAsync(req);
      router.push(`/evals/${ev.id}`);
    } catch (error) {
      console.error("Failed to create eval:", error);
    }
  };

  const canSubmit = !!formData.name;

  return (
    <div className="container mx-auto p-6">
      <Link
        href="/evals"
        className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-6"
      >
        <ArrowLeft className="w-4 h-4 mr-2" />
        {ui("Back to Evals")}
      </Link>

      <Card>
        <CardHeader>
          <CardTitle>{ui("Create New Eval")}</CardTitle>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleSubmit} className="space-y-6">
            {chatDraft.notice}
            <div className="space-y-2">
              <Label htmlFor="name">{ui("Name")}</Label>
              <Input
                id="name"
                placeholder={ui("My Eval")}
                value={formData.name}
                onChange={(e) => setFormData({ ...formData, name: e.target.value })}
                required
              />
            </div>

            <div className="space-y-2">
              <Label htmlFor="description">{ui("Description")}</Label>
              <Textarea
                id="description"
                placeholder={ui("Describe what this eval tests...")}
                value={formData.description}
                onChange={(e) => setFormData({ ...formData, description: e.target.value })}
                rows={2}
              />
            </div>

            {/* Session fields (mirrors session creation params) */}
            <div className="space-y-4 border p-4">
              <div className="space-y-2">
                <Label>{ui("Harness (optional)")}</Label>
                <HarnessSelect
                  value={harnessId}
                  onValueChange={setHarnessId}
                  placeholder={ui("Use org default")}
                  className="w-full"
                />
              </div>

              <div className="space-y-2">
                <Label>{ui("Agent (optional)")}</Label>
                <AgentSelect
                  value={agentId}
                  onValueChange={setAgentId}
                  placeholder={ui("Select an agent")}
                  className="w-full"
                />
              </div>

              <div className="space-y-2">
                <Label>{ui("Model (optional)")}</Label>
                <ModelPicker
                  value={modelId}
                  onChange={setModelId}
                  placeholder={ui("Use default")}
                />
              </div>

              <div className="space-y-2">
                <Label>{ui("System Prompt (optional)")}</Label>
                <Textarea
                  placeholder={ui("Override system prompt...")}
                  value={systemPrompt}
                  onChange={(e) => setSystemPrompt(e.target.value)}
                  rows={3}
                />
              </div>
            </div>

            <div className="space-y-2">
              <Label htmlFor="model">{ui("Model Override (optional)")}</Label>
              <ModelPicker
                value={formData.model_override}
                onChange={(value) => setFormData({ ...formData, model_override: value })}
                placeholder={ui("Use agent default")}
              />
              <p className="text-xs text-muted-foreground">
                {ui("Override the agent model for eval runs")}
              </p>
            </div>

            <div className="space-y-2">
              <Label>{ui("Tags (optional)")}</Label>
              <TagInput
                value={tags.join(", ")}
                onChange={(value) =>
                  setTags(
                    value
                      .split(",")
                      .map((tag) => tag.trim())
                      .filter(Boolean),
                  )
                }
              />
            </div>

            <div className="flex gap-4">
              <Button type="submit" disabled={createEval.isPending || !canSubmit}>
                {createEval.isPending ? ui("Creating...") : ui("Create Eval")}
              </Button>
              <Button type="button" variant="outline" onClick={() => router.back()}>
                {ui("Cancel")}
              </Button>
            </div>

            {createEval.error && (
              <p className="text-sm text-destructive">
                {ui("Error:")} {createEval.error.message}
              </p>
            )}
          </form>
        </CardContent>
      </Card>
    </div>
  );
}

export default function NewEvalPage() {
  return (
    <Suspense>
      <NewEvalForm />
    </Suspense>
  );
}
