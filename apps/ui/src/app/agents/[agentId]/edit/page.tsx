"use client";

import { useState, useEffect } from "react";
import { useParams, useRouter } from "next/navigation";
import Link from "next/link";
import { useAgent, useUpdateAgent } from "@/hooks/use-agents";
import { Header } from "@/components/layout/header";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { ArrowLeft, Loader2 } from "lucide-react";
import type { AgentStatus } from "@/lib/api/types";

export default function EditAgentPage() {
  const params = useParams();
  const router = useRouter();
  const agentId = params.agentId as string;

  const { data: agent, isLoading, error } = useAgent(agentId);
  const updateAgent = useUpdateAgent(agentId);

  const [formData, setFormData] = useState({
    name: "",
    description: "",
    default_model_id: "",
    status: "active" as AgentStatus,
  });

  useEffect(() => {
    if (agent) {
      setFormData({
        name: agent.name,
        description: agent.description || "",
        default_model_id: agent.default_model_id,
        status: agent.status,
      });
    }
  }, [agent]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();

    try {
      await updateAgent.mutateAsync({
        name: formData.name,
        description: formData.description || undefined,
        default_model_id: formData.default_model_id,
        status: formData.status,
      });
      router.push(`/agents/${agentId}`);
    } catch (error) {
      console.error("Failed to update agent:", error);
    }
  };

  if (isLoading) {
    return (
      <>
        <Header title="Edit Agent" />
        <div className="p-6 max-w-2xl">
          <Card>
            <CardHeader>
              <Skeleton className="h-6 w-32" />
              <Skeleton className="h-4 w-64" />
            </CardHeader>
            <CardContent className="space-y-6">
              {[...Array(4)].map((_, i) => (
                <Skeleton key={i} className="h-10 w-full" />
              ))}
            </CardContent>
          </Card>
        </div>
      </>
    );
  }

  if (error || !agent) {
    return (
      <>
        <Header
          title="Agent Not Found"
          action={
            <Link href="/agents">
              <Button variant="ghost">
                <ArrowLeft className="h-4 w-4 mr-2" />
                Back to Agents
              </Button>
            </Link>
          }
        />
        <div className="p-6">
          <div className="bg-destructive/10 text-destructive p-4 rounded-lg">
            {error?.message || "Agent not found"}
          </div>
        </div>
      </>
    );
  }

  return (
    <>
      <Header
        title="Edit Agent"
        action={
          <Link href={`/agents/${agentId}`}>
            <Button variant="ghost">
              <ArrowLeft className="h-4 w-4 mr-2" />
              Back to Agent
            </Button>
          </Link>
        }
      />
      <div className="p-6 max-w-2xl">
        <Card>
          <CardHeader>
            <CardTitle>Edit {agent.name}</CardTitle>
            <CardDescription>
              Update the agent's configuration and settings.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <form onSubmit={handleSubmit} className="space-y-6">
              <div className="space-y-2">
                <Label htmlFor="name">Name</Label>
                <Input
                  id="name"
                  value={formData.name}
                  onChange={(e) => setFormData({ ...formData, name: e.target.value })}
                  required
                />
              </div>

              <div className="space-y-2">
                <Label htmlFor="description">Description</Label>
                <Textarea
                  id="description"
                  value={formData.description}
                  onChange={(e) => setFormData({ ...formData, description: e.target.value })}
                  rows={3}
                />
              </div>

              <div className="space-y-2">
                <Label htmlFor="model">Default Model</Label>
                <Input
                  id="model"
                  value={formData.default_model_id}
                  onChange={(e) => setFormData({ ...formData, default_model_id: e.target.value })}
                  required
                />
              </div>

              <div className="space-y-2">
                <Label htmlFor="status">Status</Label>
                <Select
                  value={formData.status}
                  onValueChange={(value: AgentStatus) =>
                    setFormData({ ...formData, status: value })
                  }
                >
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="active">Active</SelectItem>
                    <SelectItem value="disabled">Disabled</SelectItem>
                  </SelectContent>
                </Select>
              </div>

              {updateAgent.error && (
                <div className="bg-destructive/10 text-destructive p-3 rounded-lg text-sm">
                  Failed to update agent: {updateAgent.error.message}
                </div>
              )}

              <div className="flex gap-3">
                <Button type="submit" disabled={updateAgent.isPending}>
                  {updateAgent.isPending && <Loader2 className="h-4 w-4 mr-2 animate-spin" />}
                  Save Changes
                </Button>
                <Link href={`/agents/${agentId}`}>
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
