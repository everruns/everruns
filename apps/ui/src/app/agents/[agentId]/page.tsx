"use client";

import { useState } from "react";
import { useParams } from "next/navigation";
import Link from "next/link";
import { useAgent } from "@/hooks/use-agents";
import { Header } from "@/components/layout/header";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { ArrowLeft, Bot, Settings } from "lucide-react";

export default function AgentDetailPage() {
  const params = useParams();
  const agentId = params.agentId as string;
  const [showDefinition, setShowDefinition] = useState(false);

  const {
    data: agent,
    isLoading: agentLoading,
    error: agentError,
  } = useAgent(agentId);

  if (agentLoading) {
    return (
      <>
        <Header title="Agent Details" />
        <div className="p-6 space-y-6">
          <Skeleton className="h-48" />
          <Skeleton className="h-64" />
        </div>
      </>
    );
  }

  if (agentError || !agent) {
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
            {agentError?.message || "Agent not found"}
          </div>
        </div>
      </>
    );
  }

  return (
    <>
      <Header
        title={agent.name}
        action={
          <div className="flex gap-2">
            <Link href="/agents">
              <Button variant="ghost">
                <ArrowLeft className="h-4 w-4 mr-2" />
                Back
              </Button>
            </Link>
            <Link href={`/agents/${agentId}/edit`}>
              <Button variant="outline">
                <Settings className="h-4 w-4 mr-2" />
                Edit
              </Button>
            </Link>
          </div>
        }
      />
      <div className="p-6 space-y-6">
        {/* Agent Info Card */}
        <Card>
          <CardHeader className="flex flex-row items-start justify-between">
            <div className="flex items-center gap-4">
              <div className="p-3 bg-primary/10 rounded-lg">
                <Bot className="h-8 w-8 text-primary" />
              </div>
              <div>
                <CardTitle className="text-2xl">{agent.name}</CardTitle>
                <CardDescription className="mt-1">
                  {agent.description || "No description"}
                </CardDescription>
              </div>
            </div>
            <Badge
              variant="outline"
              className={
                agent.status === "active"
                  ? "bg-green-100 text-green-800"
                  : "bg-gray-100 text-gray-800"
              }
            >
              {agent.status}
            </Badge>
          </CardHeader>
          <CardContent>
            <div className="grid grid-cols-2 md:grid-cols-3 gap-4 text-sm">
              <div>
                <p className="text-muted-foreground">Model</p>
                <p className="font-medium">{agent.default_model_id}</p>
              </div>
              <div>
                <p className="text-muted-foreground">Created</p>
                <p className="font-medium">
                  {new Date(agent.created_at).toLocaleDateString()}
                </p>
              </div>
              <div>
                <p className="text-muted-foreground">Updated</p>
                <p className="font-medium">
                  {new Date(agent.updated_at).toLocaleDateString()}
                </p>
              </div>
            </div>
          </CardContent>
        </Card>

        {/* Definition Section */}
        <Card>
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-lg">Definition</CardTitle>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setShowDefinition(!showDefinition)}
            >
              {showDefinition ? "Hide" : "Show"} Details
            </Button>
          </CardHeader>
          <CardContent>
            {agent.definition.system_prompt && (
              <div className="mb-4">
                <p className="text-sm text-muted-foreground mb-2">
                  System Prompt
                </p>
                <p className="bg-muted p-3 rounded-lg text-sm whitespace-pre-wrap">
                  {agent.definition.system_prompt}
                </p>
              </div>
            )}
            {showDefinition && (
              <div>
                <p className="text-sm text-muted-foreground mb-2">
                  Full Definition
                </p>
                <pre className="bg-muted p-4 rounded-lg text-sm overflow-auto max-h-64">
                  {JSON.stringify(agent.definition, null, 2)}
                </pre>
              </div>
            )}
          </CardContent>
        </Card>
      </div>
    </>
  );
}
