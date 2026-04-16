"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { useCreateApp } from "@/hooks/use-apps";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { AgentSelect } from "@/components/agent/agent-select";
import { AgentIdentitySelect } from "@/components/agent-identity/agent-identity-select";
import { HarnessSelect } from "@/components/harness/harness-select";
import Link from "next/link";
import { ArrowLeft } from "lucide-react";
import type { ChannelType } from "@/lib/api/types";

export default function NewAppPage() {
  const router = useRouter();
  const createApp = useCreateApp();

  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [agentId, setAgentId] = useState("");
  const [harnessId, setHarnessId] = useState("");
  const [agentIdentityId, setAgentIdentityId] = useState("");
  const [channelType, setChannelType] = useState<ChannelType>("slack");

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();

    try {
      const app = await createApp.mutateAsync({
        name,
        description: description || undefined,
        agent_id: agentId,
        agent_identity_id: agentIdentityId || undefined,
        harness_id: harnessId,
        channel_type: channelType,
        channel_config: channelType === "ag_ui" ? { anonymous: true } : undefined,
      });

      // Redirect to the detail page for channel-specific setup.
      router.push(`/apps/${app.id}`);
    } catch (error) {
      console.error("Failed to create app:", error);
    }
  };

  return (
    <div className="container mx-auto p-6">
      <Link
        href="/apps"
        className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-6"
      >
        <ArrowLeft className="w-4 h-4 mr-2" />
        Back to Apps
      </Link>

      <Card>
        <CardHeader>
          <CardTitle>Create New App</CardTitle>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleSubmit} className="space-y-6">
            <div className="space-y-2">
              <Label htmlFor="name">Name</Label>
              <Input
                id="name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="Support Bot"
                required
              />
            </div>

            <div className="space-y-2">
              <Label htmlFor="description">Description</Label>
              <Input
                id="description"
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                placeholder="Customer support bot for #help channel"
              />
            </div>

            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label htmlFor="harness">Harness</Label>
                <HarnessSelect value={harnessId} onValueChange={setHarnessId} />
              </div>

              <div className="space-y-2">
                <Label htmlFor="agent">Agent</Label>
                <AgentSelect value={agentId} onValueChange={setAgentId} />
              </div>
              <div className="space-y-2">
                <Label htmlFor="agent_identity">Agent identity</Label>
                <AgentIdentitySelect value={agentIdentityId} onValueChange={setAgentIdentityId} />
              </div>
            </div>

            <div className="space-y-2">
              <Label htmlFor="channel_type">Channel</Label>
              <Select
                value={channelType}
                onValueChange={(value) => setChannelType(value as ChannelType)}
              >
                <SelectTrigger id="channel_type">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="slack">Slack</SelectItem>
                  <SelectItem value="ag_ui">AG-UI</SelectItem>
                </SelectContent>
              </Select>
            </div>

            <p className="text-sm text-muted-foreground">
              {channelType === "slack"
                ? "After creating the app, you'll be able to generate a Slack App manifest and create the Slack bot on the detail page."
                : "After creating the app, publish it and point your AG-UI client at the channel endpoint on the detail page."}
            </p>

            <div className="flex gap-4">
              <Button
                type="submit"
                disabled={createApp.isPending || !name || !agentId || !harnessId}
              >
                {createApp.isPending ? "Creating..." : "Create App"}
              </Button>
              <Button type="button" variant="outline" onClick={() => router.back()}>
                Cancel
              </Button>
            </div>

            {createApp.error && (
              <p className="text-sm text-destructive">Error: {createApp.error.message}</p>
            )}
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
