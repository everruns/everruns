"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { useCreateEval } from "@/hooks";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Badge } from "@/components/ui/badge";
import Link from "next/link";
import { ArrowLeft, X } from "lucide-react";
import { ModelPicker } from "@/components/models/model-picker";
import { AgentSelect } from "@/components/agent/agent-select";
import { HarnessSelect } from "@/components/harness/harness-select";

export default function NewEvalPage() {
  const router = useRouter();
  const createEval = useCreateEval();
  const [formData, setFormData] = useState({
    name: "",
    description: "",
    agent_id: "",
    harness_id: "",
    model_override: "",
  });
  const [tags, setTags] = useState<string[]>([]);
  const [tagInput, setTagInput] = useState("");

  const handleAddTag = () => {
    const tag = tagInput.trim();
    if (tag && !tags.includes(tag)) {
      setTags([...tags, tag]);
      setTagInput("");
    }
  };

  const handleRemoveTag = (tag: string) => {
    setTags(tags.filter((t) => t !== tag));
  };

  const handleTagKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      handleAddTag();
    }
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();

    try {
      const ev = await createEval.mutateAsync({
        name: formData.name,
        description: formData.description || undefined,
        agent_id: formData.agent_id,
        harness_id: formData.harness_id,
        model_override: formData.model_override || undefined,
        tags: tags.length > 0 ? tags : undefined,
      });

      router.push(`/evals/${ev.id}`);
    } catch (error) {
      console.error("Failed to create eval:", error);
    }
  };

  return (
    <div className="container mx-auto p-6">
      <Link
        href="/evals"
        className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-6"
      >
        <ArrowLeft className="w-4 h-4 mr-2" />
        Back to Evals
      </Link>

      <Card>
        <CardHeader>
          <CardTitle>Create New Eval</CardTitle>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleSubmit} className="space-y-6">
            <div className="space-y-2">
              <Label htmlFor="name">Name</Label>
              <Input
                id="name"
                placeholder="My Eval"
                value={formData.name}
                onChange={(e) => setFormData({ ...formData, name: e.target.value })}
                required
              />
            </div>

            <div className="space-y-2">
              <Label htmlFor="description">Description</Label>
              <Textarea
                id="description"
                placeholder="Describe what this eval tests..."
                value={formData.description}
                onChange={(e) => setFormData({ ...formData, description: e.target.value })}
                rows={2}
              />
            </div>

            <div className="space-y-2">
              <Label htmlFor="agent">Agent</Label>
              <AgentSelect
                value={formData.agent_id}
                onValueChange={(value) => setFormData({ ...formData, agent_id: value })}
                placeholder="Select an agent"
                className="w-full"
              />
            </div>

            <div className="space-y-2">
              <Label htmlFor="harness">Harness</Label>
              <HarnessSelect
                value={formData.harness_id}
                onValueChange={(value) => setFormData({ ...formData, harness_id: value })}
                placeholder="Select a harness"
                className="w-full"
              />
            </div>

            <div className="space-y-2">
              <Label htmlFor="model">Model Override (optional)</Label>
              <ModelPicker
                value={formData.model_override}
                onChange={(value) => setFormData({ ...formData, model_override: value })}
                placeholder="Use agent default"
              />
              <p className="text-xs text-muted-foreground">
                Override the agent model for eval runs
              </p>
            </div>

            <div className="space-y-2">
              <Label>Tags (optional)</Label>
              <div className="flex gap-2">
                <Input
                  placeholder="Add a tag..."
                  value={tagInput}
                  onChange={(e) => setTagInput(e.target.value)}
                  onKeyDown={handleTagKeyDown}
                />
                <Button type="button" variant="outline" onClick={handleAddTag}>
                  Add
                </Button>
              </div>
              {tags.length > 0 && (
                <div className="flex flex-wrap gap-1 mt-2">
                  {tags.map((tag) => (
                    <Badge key={tag} variant="secondary" className="flex items-center gap-1">
                      {tag}
                      <button type="button" onClick={() => handleRemoveTag(tag)}>
                        <X className="w-3 h-3" />
                      </button>
                    </Badge>
                  ))}
                </div>
              )}
            </div>

            <div className="flex gap-4">
              <Button
                type="submit"
                disabled={createEval.isPending || !formData.agent_id || !formData.harness_id}
              >
                {createEval.isPending ? "Creating..." : "Create Eval"}
              </Button>
              <Button type="button" variant="outline" onClick={() => router.back()}>
                Cancel
              </Button>
            </div>

            {createEval.error && (
              <p className="text-sm text-destructive">Error: {createEval.error.message}</p>
            )}
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
