"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { useCreateAgent } from "@/hooks/use-agents";
import { Header } from "@/components/layout/header";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Label } from "@/components/ui/label";
import { ArrowLeft, Loader2 } from "lucide-react";
import Link from "next/link";

export default function NewAgentPage() {
  const router = useRouter();
  const createAgent = useCreateAgent();

  const [formData, setFormData] = useState({
    name: "",
    description: "",
    default_model_id: "gpt-5.1",
    system_prompt: "",
  });

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();

    try {
      const agent = await createAgent.mutateAsync({
        name: formData.name,
        description: formData.description || undefined,
        default_model_id: formData.default_model_id,
        definition: {
          system_prompt: formData.system_prompt || undefined,
        },
      });
      router.push(`/agents/${agent.id}`);
    } catch (error) {
      console.error("Failed to create agent:", error);
    }
  };

  return (
    <>
      <Header
        title="Create Agent"
        action={
          <Link href="/agents">
            <Button variant="ghost">
              <ArrowLeft className="h-4 w-4 mr-2" />
              Back to Agents
            </Button>
          </Link>
        }
      />
      <div className="p-6 max-w-2xl">
        <Card>
          <CardHeader>
            <CardTitle>New Agent</CardTitle>
            <CardDescription>
              Create a new AI agent with a specific configuration.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <form onSubmit={handleSubmit} className="space-y-6">
              <div className="space-y-2">
                <Label htmlFor="name">Name</Label>
                <Input
                  id="name"
                  placeholder="My Assistant"
                  value={formData.name}
                  onChange={(e) => setFormData({ ...formData, name: e.target.value })}
                  required
                />
              </div>

              <div className="space-y-2">
                <Label htmlFor="description">Description</Label>
                <Textarea
                  id="description"
                  placeholder="A helpful assistant that..."
                  value={formData.description}
                  onChange={(e) => setFormData({ ...formData, description: e.target.value })}
                  rows={3}
                />
              </div>

              <div className="space-y-2">
                <Label htmlFor="model">Default Model</Label>
                <Input
                  id="model"
                  placeholder="gpt-5.1"
                  value={formData.default_model_id}
                  onChange={(e) => setFormData({ ...formData, default_model_id: e.target.value })}
                  required
                />
                <p className="text-xs text-muted-foreground">
                  The model ID to use for this agent (e.g., gpt-5.1, gpt-4o)
                </p>
              </div>

              <div className="space-y-2">
                <Label htmlFor="system_prompt">System Prompt</Label>
                <Textarea
                  id="system_prompt"
                  placeholder="You are a helpful assistant..."
                  value={formData.system_prompt}
                  onChange={(e) => setFormData({ ...formData, system_prompt: e.target.value })}
                  rows={4}
                />
                <p className="text-xs text-muted-foreground">
                  Instructions that define how the agent behaves
                </p>
              </div>

              {createAgent.error && (
                <div className="bg-destructive/10 text-destructive p-3 rounded-lg text-sm">
                  Failed to create agent: {createAgent.error.message}
                </div>
              )}

              <div className="flex gap-3">
                <Button type="submit" disabled={createAgent.isPending}>
                  {createAgent.isPending && <Loader2 className="h-4 w-4 mr-2 animate-spin" />}
                  Create Agent
                </Button>
                <Link href="/agents">
                  <Button type="button" variant="outline">
                    Cancel
                  </Button>
                </Link>
              </div>
            </form>
          </CardContent>
        </Card>
      </div>
    </>
  );
}
