"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { useCreateHarness } from "@/hooks";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import Link from "next/link";
import { ArrowLeft } from "lucide-react";

export default function NewHarnessPage() {
  const router = useRouter();
  const createHarness = useCreateHarness();

  const [formData, setFormData] = useState({
    slug: "",
    display_name: "",
    description: "",
    system_prompt: "",
    temperature: "",
    max_tokens: "",
    tags: "",
  });

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();

    try {
      const harness = await createHarness.mutateAsync({
        slug: formData.slug,
        display_name: formData.display_name,
        description: formData.description || undefined,
        system_prompt: formData.system_prompt,
        temperature: formData.temperature
          ? parseFloat(formData.temperature)
          : undefined,
        max_tokens: formData.max_tokens
          ? parseInt(formData.max_tokens)
          : undefined,
        tags: formData.tags
          ? formData.tags.split(",").map((t) => t.trim())
          : [],
      });

      router.push(`/harnesses/${harness.id}`);
    } catch (error) {
      console.error("Failed to create harness:", error);
    }
  };

  return (
    <div className="container mx-auto p-6 max-w-2xl">
      <Link
        href="/harnesses"
        className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-6"
      >
        <ArrowLeft className="w-4 h-4 mr-2" />
        Back to Harnesses
      </Link>

      <Card>
        <CardHeader>
          <CardTitle>Create New Harness</CardTitle>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleSubmit} className="space-y-6">
            <div className="grid gap-4 md:grid-cols-2">
              <div className="space-y-2">
                <Label htmlFor="slug">Slug</Label>
                <Input
                  id="slug"
                  placeholder="my-harness"
                  value={formData.slug}
                  onChange={(e) =>
                    setFormData({ ...formData, slug: e.target.value })
                  }
                  required
                />
                <p className="text-xs text-muted-foreground">
                  Unique identifier for the harness
                </p>
              </div>

              <div className="space-y-2">
                <Label htmlFor="display_name">Display Name</Label>
                <Input
                  id="display_name"
                  placeholder="My Harness"
                  value={formData.display_name}
                  onChange={(e) =>
                    setFormData({ ...formData, display_name: e.target.value })
                  }
                  required
                />
              </div>
            </div>

            <div className="space-y-2">
              <Label htmlFor="description">Description</Label>
              <Textarea
                id="description"
                placeholder="Describe what this harness does..."
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

            <div className="grid gap-4 md:grid-cols-2">
              <div className="space-y-2">
                <Label htmlFor="temperature">Temperature</Label>
                <Input
                  id="temperature"
                  type="number"
                  step="0.1"
                  min="0"
                  max="2"
                  placeholder="0.7"
                  value={formData.temperature}
                  onChange={(e) =>
                    setFormData({ ...formData, temperature: e.target.value })
                  }
                />
              </div>

              <div className="space-y-2">
                <Label htmlFor="max_tokens">Max Tokens</Label>
                <Input
                  id="max_tokens"
                  type="number"
                  min="1"
                  placeholder="4096"
                  value={formData.max_tokens}
                  onChange={(e) =>
                    setFormData({ ...formData, max_tokens: e.target.value })
                  }
                />
              </div>
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
              <Button type="submit" disabled={createHarness.isPending}>
                {createHarness.isPending ? "Creating..." : "Create Harness"}
              </Button>
              <Button type="button" variant="outline" onClick={() => router.back()}>
                Cancel
              </Button>
            </div>

            {createHarness.error && (
              <p className="text-sm text-destructive">
                Error: {createHarness.error.message}
              </p>
            )}
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
