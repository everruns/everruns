"use client";

import Link from "next/link";
import { useAgents } from "@/hooks/use-agents";
import { Header } from "@/components/layout/header";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { Plus, Bot, Settings } from "lucide-react";
import type { Agent } from "@/lib/api/types";

function AgentCard({ agent }: { agent: Agent }) {
  return (
    <Card className="hover:shadow-md transition-shadow">
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <div className="flex items-center gap-3">
          <div className="p-2 bg-primary/10 rounded-lg">
            <Bot className="h-5 w-5 text-primary" />
          </div>
          <div>
            <CardTitle className="text-lg">{agent.name}</CardTitle>
            <CardDescription className="text-sm">
              {agent.default_model_id}
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
        <p className="text-sm text-muted-foreground mb-4">
          {agent.description || "No description provided"}
        </p>
        <div className="flex items-center justify-between text-xs text-muted-foreground">
          <span>Created {new Date(agent.created_at).toLocaleDateString()}</span>
          <div className="flex gap-2">
            <Link href={`/agents/${agent.id}`}>
              <Button variant="outline" size="sm">
                View
              </Button>
            </Link>
            <Link href={`/agents/${agent.id}/edit`}>
              <Button variant="ghost" size="sm">
                <Settings className="h-4 w-4" />
              </Button>
            </Link>
          </div>
        </div>
      </CardContent>
    </Card>
  );
}

function AgentCardSkeleton() {
  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <div className="flex items-center gap-3">
          <Skeleton className="h-9 w-9 rounded-lg" />
          <div className="space-y-2">
            <Skeleton className="h-5 w-32" />
            <Skeleton className="h-4 w-24" />
          </div>
        </div>
        <Skeleton className="h-5 w-16" />
      </CardHeader>
      <CardContent>
        <Skeleton className="h-4 w-full mb-4" />
        <div className="flex items-center justify-between">
          <Skeleton className="h-4 w-24" />
          <Skeleton className="h-8 w-20" />
        </div>
      </CardContent>
    </Card>
  );
}

export default function AgentsPage() {
  const { data: agents = [], isLoading, error } = useAgents();

  return (
    <>
      <Header
        title="Agents"
        action={
          <Link href="/agents/new">
            <Button>
              <Plus className="h-4 w-4 mr-2" />
              New Agent
            </Button>
          </Link>
        }
      />
      <div className="p-6">
        {error && (
          <div className="bg-destructive/10 text-destructive p-4 rounded-lg mb-6">
            Failed to load agents: {error.message}
          </div>
        )}

        {isLoading ? (
          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
            {[...Array(6)].map((_, i) => (
              <AgentCardSkeleton key={i} />
            ))}
          </div>
        ) : agents.length === 0 ? (
          <div className="text-center py-12">
            <Bot className="h-16 w-16 mx-auto text-muted-foreground mb-4" />
            <h3 className="text-lg font-medium mb-2">No agents yet</h3>
            <p className="text-muted-foreground mb-4">
              Create your first agent to get started
            </p>
            <Link href="/agents/new">
              <Button>
                <Plus className="h-4 w-4 mr-2" />
                Create Agent
              </Button>
            </Link>
          </div>
        ) : (
          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
            {agents.map((agent) => (
              <AgentCard key={agent.id} agent={agent} />
            ))}
          </div>
        )}
      </div>
    </>
  );
}
