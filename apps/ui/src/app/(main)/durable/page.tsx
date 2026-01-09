"use client";

import { Header } from "@/components/layout/header";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { useDurableHealth, useWorkers, useWorkflows } from "@/hooks";
import {
  Server,
  Workflow,
  Clock,
  AlertTriangle,
  CheckCircle,
  Activity,
  Inbox,
  Zap,
} from "lucide-react";
import Link from "next/link";
import { Button } from "@/components/ui/button";

function getHealthStatusColor(status: string) {
  switch (status) {
    case "healthy":
      return "bg-green-500";
    case "degraded":
      return "bg-yellow-500";
    case "unhealthy":
      return "bg-red-500";
    default:
      return "bg-gray-500";
  }
}

function getHealthBadgeVariant(status: string) {
  switch (status) {
    case "healthy":
      return "default" as const;
    case "degraded":
      return "secondary" as const;
    case "unhealthy":
      return "destructive" as const;
    default:
      return "outline" as const;
  }
}

export default function DurableDashboardPage() {
  const { data: health, isLoading: healthLoading, error: healthError } = useDurableHealth();
  const { data: workersData, isLoading: workersLoading } = useWorkers();
  const { data: workflowsData, isLoading: workflowsLoading } = useWorkflows({ limit: 5 });

  const isLoading = healthLoading || workersLoading || workflowsLoading;

  if (isLoading) {
    return (
      <>
        <Header title="Durable Execution" />
        <div className="p-6 space-y-6">
          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
            {[...Array(4)].map((_, i) => (
              <Skeleton key={i} className="h-32" />
            ))}
          </div>
          <div className="grid gap-6 md:grid-cols-2">
            <Skeleton className="h-80" />
            <Skeleton className="h-80" />
          </div>
        </div>
      </>
    );
  }

  // Handle error state - show empty state when API not available
  if (healthError) {
    return (
      <>
        <Header title="Durable Execution" />
        <div className="p-6">
          <Card>
            <CardContent className="flex flex-col items-center justify-center py-12">
              <AlertTriangle className="h-12 w-12 text-muted-foreground mb-4" />
              <h3 className="text-lg font-medium mb-2">Durable API Not Available</h3>
              <p className="text-sm text-muted-foreground text-center max-w-md">
                The durable execution API endpoints are not yet available.
                This dashboard will show worker and workflow information once the backend is ready.
              </p>
            </CardContent>
          </Card>
        </div>
      </>
    );
  }

  const stats = [
    {
      title: "System Health",
      value: health?.status || "unknown",
      description: `Load: ${health?.load_percentage?.toFixed(1) || 0}%`,
      icon: Activity,
      color: getHealthStatusColor(health?.status || "unknown"),
      isBadge: true,
    },
    {
      title: "Active Workers",
      value: health?.active_workers || 0,
      description: `${health?.workers_accepting || 0} accepting tasks`,
      icon: Server,
      color: "text-blue-600",
    },
    {
      title: "Running Workflows",
      value: health?.running_workflows || 0,
      description: `${health?.pending_workflows || 0} pending`,
      icon: Workflow,
      color: "text-green-600",
    },
    {
      title: "Pending Tasks",
      value: health?.pending_tasks || 0,
      description: `${health?.claimed_tasks || 0} claimed`,
      icon: Clock,
      color: "text-yellow-600",
    },
  ];

  return (
    <>
      <Header title="Durable Execution" />
      <div className="p-6 space-y-6">
        {/* Stats Cards */}
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
          {stats.map((stat) => (
            <Card key={stat.title}>
              <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                <CardTitle className="text-sm font-medium">{stat.title}</CardTitle>
                <stat.icon className={`h-4 w-4 ${stat.color}`} />
              </CardHeader>
              <CardContent>
                {stat.isBadge ? (
                  <Badge variant={getHealthBadgeVariant(String(stat.value))} className="text-lg px-3 py-1">
                    {String(stat.value).toUpperCase()}
                  </Badge>
                ) : (
                  <div className="text-2xl font-bold">{stat.value}</div>
                )}
                <p className="text-xs text-muted-foreground mt-1">{stat.description}</p>
              </CardContent>
            </Card>
          ))}
        </div>

        {/* Alert for issues */}
        {health && (health.dlq_size > 0 || health.open_circuit_breakers.length > 0) && (
          <Card className="border-yellow-500/50 bg-yellow-500/5">
            <CardHeader className="pb-2">
              <div className="flex items-center gap-2">
                <AlertTriangle className="h-5 w-5 text-yellow-600" />
                <CardTitle className="text-base">Attention Required</CardTitle>
              </div>
            </CardHeader>
            <CardContent className="space-y-2">
              {health.dlq_size > 0 && (
                <div className="flex items-center justify-between">
                  <span className="text-sm">
                    <Inbox className="h-4 w-4 inline mr-2" />
                    {health.dlq_size} items in dead letter queue
                  </span>
                  <Link href="/durable/workflows?tab=dlq">
                    <Button variant="outline" size="sm">View DLQ</Button>
                  </Link>
                </div>
              )}
              {health.open_circuit_breakers.length > 0 && (
                <div className="flex items-center justify-between">
                  <span className="text-sm">
                    <Zap className="h-4 w-4 inline mr-2" />
                    {health.open_circuit_breakers.length} circuit breakers open: {health.open_circuit_breakers.join(", ")}
                  </span>
                </div>
              )}
            </CardContent>
          </Card>
        )}

        {/* Main content grid */}
        <div className="grid gap-6 md:grid-cols-2">
          {/* Workers Summary */}
          <Card>
            <CardHeader className="flex flex-row items-center justify-between">
              <div>
                <CardTitle>Workers</CardTitle>
                <CardDescription>Active worker pool status</CardDescription>
              </div>
              <Link href="/durable/workers">
                <Button variant="outline" size="sm">View All</Button>
              </Link>
            </CardHeader>
            <CardContent>
              {workersData && workersData.workers.length > 0 ? (
                <div className="space-y-4">
                  {/* Capacity bar */}
                  <div>
                    <div className="flex justify-between text-sm mb-1">
                      <span className="text-muted-foreground">Capacity Usage</span>
                      <span className="font-medium">
                        {workersData.summary.total_load} / {workersData.summary.total_capacity}
                      </span>
                    </div>
                    <div className="h-2 bg-muted rounded-full overflow-hidden">
                      <div
                        className="h-full bg-primary transition-all"
                        style={{
                          width: `${workersData.summary.total_capacity > 0
                            ? (workersData.summary.total_load / workersData.summary.total_capacity) * 100
                            : 0}%`,
                        }}
                      />
                    </div>
                  </div>

                  {/* Worker list */}
                  <div className="space-y-2">
                    {workersData.workers.slice(0, 5).map((worker) => (
                      <div
                        key={worker.id}
                        className="flex items-center justify-between p-2 rounded-lg bg-muted/50"
                      >
                        <div className="flex items-center gap-2">
                          <div
                            className={`w-2 h-2 rounded-full ${
                              worker.status === "active" ? "bg-green-500" :
                              worker.status === "draining" ? "bg-yellow-500" : "bg-red-500"
                            }`}
                          />
                          <span className="text-sm font-medium truncate max-w-[150px]">
                            {worker.hostname || worker.id.slice(0, 12)}
                          </span>
                        </div>
                        <div className="flex items-center gap-2">
                          <span className="text-xs text-muted-foreground">
                            {worker.current_load}/{worker.max_concurrency}
                          </span>
                          <Badge variant={worker.accepting_tasks ? "default" : "secondary"} className="text-xs">
                            {worker.accepting_tasks ? "accepting" : "busy"}
                          </Badge>
                        </div>
                      </div>
                    ))}
                  </div>
                </div>
              ) : (
                <div className="flex flex-col items-center justify-center py-8 text-muted-foreground">
                  <Server className="h-8 w-8 mb-2" />
                  <p className="text-sm">No workers connected</p>
                </div>
              )}
            </CardContent>
          </Card>

          {/* Recent Workflows */}
          <Card>
            <CardHeader className="flex flex-row items-center justify-between">
              <div>
                <CardTitle>Recent Workflows</CardTitle>
                <CardDescription>Latest workflow executions</CardDescription>
              </div>
              <Link href="/durable/workflows">
                <Button variant="outline" size="sm">View All</Button>
              </Link>
            </CardHeader>
            <CardContent>
              {workflowsData && workflowsData.data.length > 0 ? (
                <div className="space-y-2">
                  {workflowsData.data.map((workflow) => (
                    <Link
                      key={workflow.id}
                      href={`/durable/workflows/${workflow.id}`}
                      className="flex items-center justify-between p-2 rounded-lg bg-muted/50 hover:bg-muted transition-colors"
                    >
                      <div className="flex items-center gap-2">
                        <WorkflowStatusIcon status={workflow.status} />
                        <div>
                          <p className="text-sm font-medium">{workflow.workflow_type}</p>
                          <p className="text-xs text-muted-foreground">
                            {workflow.id.slice(0, 8)}...
                          </p>
                        </div>
                      </div>
                      <Badge variant={getWorkflowStatusVariant(workflow.status)}>
                        {workflow.status}
                      </Badge>
                    </Link>
                  ))}
                </div>
              ) : (
                <div className="flex flex-col items-center justify-center py-8 text-muted-foreground">
                  <Workflow className="h-8 w-8 mb-2" />
                  <p className="text-sm">No workflows yet</p>
                </div>
              )}
            </CardContent>
          </Card>
        </div>

        {/* Task Queue by Type */}
        {health && Object.keys(health.queue_depth_by_type).length > 0 && (
          <Card>
            <CardHeader>
              <CardTitle>Task Queue by Type</CardTitle>
              <CardDescription>Pending tasks grouped by activity type</CardDescription>
            </CardHeader>
            <CardContent>
              <div className="grid gap-2 md:grid-cols-3 lg:grid-cols-4">
                {Object.entries(health.queue_depth_by_type).map(([type, count]) => (
                  <div key={type} className="flex items-center justify-between p-3 rounded-lg bg-muted/50">
                    <span className="text-sm font-medium">{type}</span>
                    <Badge variant="outline">{count}</Badge>
                  </div>
                ))}
              </div>
            </CardContent>
          </Card>
        )}
      </div>
    </>
  );
}

function WorkflowStatusIcon({ status }: { status: string }) {
  switch (status) {
    case "completed":
      return <CheckCircle className="h-4 w-4 text-green-500" />;
    case "running":
      return <Activity className="h-4 w-4 text-blue-500 animate-pulse" />;
    case "failed":
      return <AlertTriangle className="h-4 w-4 text-red-500" />;
    case "cancelled":
      return <AlertTriangle className="h-4 w-4 text-yellow-500" />;
    default:
      return <Clock className="h-4 w-4 text-gray-500" />;
  }
}

function getWorkflowStatusVariant(status: string) {
  switch (status) {
    case "completed":
      return "default" as const;
    case "running":
      return "secondary" as const;
    case "failed":
      return "destructive" as const;
    case "cancelled":
      return "outline" as const;
    default:
      return "outline" as const;
  }
}
