"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { useCreateAgent, useLlmModels } from "@/hooks";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { PromptEditor } from "@/components/ui/prompt-editor";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import Link from "next/link";
import { ArrowLeft } from "lucide-react";

export default function NewAgentPage() {
  const router = useRouter();
  const createAgent = useCreateAgent();
  const { data: models = [] } = useLlmModels();

  const [formData, setFormData] = useState({
    name: "",
    description: "",
    system_prompt: "",
    default_model_id: "",
  });

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();

    try {
      const agent = await createAgent.mutateAsync({
        name: formData.name,
        description: formData.description || undefined,
        system_prompt: formData.system_prompt,
        default_model_id: formData.default_model_id || undefined,
      });

      router.push(`/agents/${agent.id}`);
    } catch (error) {
      console.error("Failed to create agent:", error);
    }
  };

  return (
    <div className="container mx-auto p-6 max-w-2xl">
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
              <Label htmlFor="name">Name</Label>
              <Input
                id="name"
                placeholder="My Agent"
                value={formData.name}
                onChange={(e) =>
                  setFormData({ ...formData, name: e.target.value })
                }
                required
              />
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
              <Select
                value={formData.default_model_id}
                onValueChange={(value) =>
                  setFormData({ ...formData, default_model_id: value === "none" ? "" : value })
                }
              >
                <SelectTrigger className="w-full">
                  <SelectValue placeholder="Use default model" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="none">Use default model</SelectItem>
                  {models.map((model) => (
                    <SelectItem key={model.id} value={model.id}>
                      {model.display_name} ({model.provider_name})
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <p className="text-xs text-muted-foreground">
                Select a specific model or leave empty to use the default
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

            <div className="flex gap-4">
              <Button type="submit" disabled={createAgent.isPending}>
                {createAgent.isPending ? "Creating..." : "Create Agent"}
              </Button>
              <Button type="button" variant="outline" onClick={() => router.back()}>
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
