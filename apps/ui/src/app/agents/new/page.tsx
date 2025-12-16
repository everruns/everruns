"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { useCreateAgent } from "@/hooks";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import Link from "next/link";
import { ArrowLeft } from "lucide-react";

export default function NewAgentPage() {
  const router = useRouter();
  const createAgent = useCreateAgent();

  const [formData, setFormData] = useState({
    name: "",
    description: "",
    system_prompt: "",
    tags: "",
  });

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();

    try {
      const agent = await createAgent.mutateAsync({
        name: formData.name,
        description: formData.description || undefined,
        system_prompt: formData.system_prompt,
        tags: formData.tags
          ? formData.tags.split(",").map((t) => t.trim())
          : [],
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
              <Label htmlFor="system_prompt">System Prompt</Label>
              <Textarea
                id="system_prompt"
                placeholder="You are a helpful assistant..."
                value={formData.system_prompt}
                onChange={(e) =>
                  setFormData({ ...formData, system_prompt: e.target.value })
                }
                rows={6}
                required
              />
              <p className="text-xs text-muted-foreground">
                Instructions for the AI model
              </p>
            </div>

            <div className="space-y-2">
              <Label htmlFor="tags">Tags</Label>
              <Input
                id="tags"
                placeholder="tag1, tag2, tag3"
                value={formData.tags}
                onChange={(e) =>
                  setFormData({ ...formData, tags: e.target.value })
                }
              />
              <p className="text-xs text-muted-foreground">
                Comma-separated list of tags
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
