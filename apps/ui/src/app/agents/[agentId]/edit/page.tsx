"use client";

import { useState, useEffect, useMemo } from "react";
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

  const initialFormData = useMemo(() => ({
    name: agent?.name || "",
    description: agent?.description || "",
    default_model_id: agent?.default_model_id || "",
    status: (agent?.status || "active") as AgentStatus,
  }), [agent]);

  const [formData, setFormData] = useState(initialFormData);

  // Sync form data when agent loads - valid pattern for external data sync
  useEffect(() => {
    if (agent) {
      // eslint-disable-next-line react-hooks/set-state-in-effect
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
              <Skeleton className="h-8 w-48" />
              <Skeleton className="h-4 w-64" />
            </CardHeader>
            <CardContent className="space-y-4">
              <Skeleton className="h-10 w-full" />
              <Skeleton className="h-20 w-full" />
              <Skeleton className="h-10 w-full" />
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
          title="Edit Agent"
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
              Update the agent&apos;s configuration and settings.
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
                  placeholder="Optional description for your agent"
                />
              </div>

              <div className="space-y-2">
                <Label htmlFor="model">Model ID</Label>
                <Input
                  id="model"
                  value={formData.default_model_id}
                  onChange={(e) => setFormData({ ...formData, default_model_id: e.target.value })}
                  placeholder="gpt-4"
                  required
                />
              </div>

              <div className="space-y-2">
                <Label htmlFor="status">Status</Label>
                <Select
                  value={formData.status}
                  onValueChange={(value: AgentStatus) => setFormData({ ...formData, status: value })}
                >
                  <SelectTrigger>
                    <SelectValue placeholder="Select status" />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="active">Active</SelectItem>
                    <SelectItem value="disabled">Disabled</SelectItem>
                  </SelectContent>
                </Select>
              </div>

              <div className="flex gap-2">
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
