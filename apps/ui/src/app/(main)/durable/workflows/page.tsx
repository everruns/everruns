"use client";

import { useState } from "react";
import { Header } from "@/components/layout/header";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useWorkflows, useCancelWorkflow, useTasks, useDlq, useRequeueDlqEntry } from "@/hooks";
import type { DurableWorkflow, WorkflowStatus, DurableTask, DlqEntry } from "@/lib/api/types";
import {
  Workflow,
  AlertTriangle,
  CheckCircle,
  Clock,
  XCircle,
  Activity,
  RefreshCw,
  Search,
  ExternalLink,
  RotateCcw,
  Inbox,
  ListTodo,
} from "lucide-react";
import Link from "next/link";
import { cn } from "@/lib/utils";

type TabValue = "workflows" | "tasks" | "dlq";

function formatDistanceToNow(date: Date, options?: { addSuffix?: boolean }): string {
  const now = new Date();
  const diff = now.getTime() - date.getTime();
  const seconds = Math.floor(diff / 1000);
  const minutes = Math.floor(seconds / 60);
  const hours = Math.floor(minutes / 60);
  const days = Math.floor(hours / 24);

  let result = "";
  if (days > 0) {
    result = `${days} day${days > 1 ? "s" : ""}`;
  } else if (hours > 0) {
    result = `${hours} hour${hours > 1 ? "s" : ""}`;
  } else if (minutes > 0) {
    result = `${minutes} minute${minutes > 1 ? "s" : ""}`;
  } else {
    result = "less than a minute";
  }

  return options?.addSuffix ? `${result} ago` : result;
}

function getStatusIcon(status: WorkflowStatus) {
  switch (status) {
    case "completed":
      return <CheckCircle className="h-4 w-4 text-green-500" />;
    case "running":
      return <Activity className="h-4 w-4 text-blue-500 animate-pulse" />;
    case "failed":
      return <XCircle className="h-4 w-4 text-red-500" />;
    case "cancelled":
      return <AlertTriangle className="h-4 w-4 text-yellow-500" />;
    default:
      return <Clock className="h-4 w-4 text-gray-500" />;
  }
}

function getStatusBadgeVariant(status: WorkflowStatus) {
  switch (status) {
    case "completed":
      return "default" as const;
    case "running":
      return "secondary" as const;
    case "failed":
      return "destructive" as const;
    case "cancelled":
    case "pending":
      return "outline" as const;
    default:
      return "outline" as const;
  }
}

function WorkflowRow({ workflow, onCancel }: { workflow: DurableWorkflow; onCancel: (id: string) => void }) {
  return (
    <TableRow>
      <TableCell>
        <div className="flex items-center gap-2">
          {getStatusIcon(workflow.status)}
          <div>
            <p className="font-medium">{workflow.workflow_type}</p>
            <p className="text-xs text-muted-foreground font-mono">{workflow.id.slice(0, 16)}...</p>
          </div>
        </div>
      </TableCell>
      <TableCell>
        <Badge variant={getStatusBadgeVariant(workflow.status)}>{workflow.status}</Badge>
      </TableCell>
      <TableCell>
        <TooltipProvider>
          <Tooltip>
            <TooltipTrigger className="text-sm text-muted-foreground">
              {formatDistanceToNow(new Date(workflow.created_at), { addSuffix: true })}
            </TooltipTrigger>
            <TooltipContent>
              {new Date(workflow.created_at).toLocaleString()}
            </TooltipContent>
          </Tooltip>
        </TooltipProvider>
      </TableCell>
      <TableCell>
        {workflow.started_at ? (
          <TooltipProvider>
            <Tooltip>
              <TooltipTrigger className="text-sm text-muted-foreground">
                {formatDistanceToNow(new Date(workflow.started_at), { addSuffix: true })}
              </TooltipTrigger>
              <TooltipContent>
                {new Date(workflow.started_at).toLocaleString()}
              </TooltipContent>
            </Tooltip>
          </TooltipProvider>
        ) : (
          <span className="text-sm text-muted-foreground">-</span>
        )}
      </TableCell>
      <TableCell>
        {workflow.completed_at ? (
          <TooltipProvider>
            <Tooltip>
              <TooltipTrigger className="text-sm text-muted-foreground">
                {formatDistanceToNow(new Date(workflow.completed_at), { addSuffix: true })}
              </TooltipTrigger>
              <TooltipContent>
                {new Date(workflow.completed_at).toLocaleString()}
              </TooltipContent>
            </Tooltip>
          </TooltipProvider>
        ) : (
          <span className="text-sm text-muted-foreground">-</span>
        )}
      </TableCell>
      <TableCell>
        {workflow.session_id ? (
          <Link
            href={`/agents/${workflow.agent_id}/sessions/${workflow.session_id}`}
            className="flex items-center gap-1 text-sm text-primary hover:underline"
          >
            <ExternalLink className="h-3 w-3" />
            View Session
          </Link>
        ) : (
          <span className="text-sm text-muted-foreground">-</span>
        )}
      </TableCell>
      <TableCell>
        <div className="flex items-center gap-2">
          <Link href={`/durable/workflows/${workflow.id}`}>
            <Button variant="outline" size="sm">
              View
            </Button>
          </Link>
          {workflow.status === "running" && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => onCancel(workflow.id)}
            >
              Cancel
            </Button>
          )}
        </div>
      </TableCell>
    </TableRow>
  );
}

function TaskRow({ task }: { task: DurableTask }) {
  return (
    <TableRow>
      <TableCell>
        <div>
          <p className="font-medium">{task.activity_type}</p>
          <p className="text-xs text-muted-foreground">{task.activity_id}</p>
        </div>
      </TableCell>
      <TableCell>
        <Badge variant={task.status === "pending" ? "outline" : task.status === "claimed" ? "secondary" : "default"}>
          {task.status}
        </Badge>
      </TableCell>
      <TableCell>
        <Badge variant="outline">{task.priority}</Badge>
      </TableCell>
      <TableCell>
        <span className="text-sm">{task.attempt}/{task.max_attempts}</span>
      </TableCell>
      <TableCell>
        {task.claimed_by ? (
          <span className="text-sm font-mono text-muted-foreground">
            {task.claimed_by.slice(0, 12)}...
          </span>
        ) : (
          <span className="text-sm text-muted-foreground">-</span>
        )}
      </TableCell>
      <TableCell>
        <TooltipProvider>
          <Tooltip>
            <TooltipTrigger className="text-sm text-muted-foreground">
              {formatDistanceToNow(new Date(task.scheduled_at), { addSuffix: true })}
            </TooltipTrigger>
            <TooltipContent>
              {new Date(task.scheduled_at).toLocaleString()}
            </TooltipContent>
          </Tooltip>
        </TooltipProvider>
      </TableCell>
      <TableCell>
        <Link href={`/durable/workflows/${task.workflow_id}`}>
          <Button variant="ghost" size="sm">
            <ExternalLink className="h-3 w-3" />
          </Button>
        </Link>
      </TableCell>
    </TableRow>
  );
}

function DlqRow({ entry, onRequeue }: { entry: DlqEntry; onRequeue: (id: string) => void }) {
  return (
    <TableRow>
      <TableCell>
        <div>
          <p className="font-medium">{entry.activity_type}</p>
          <p className="text-xs text-muted-foreground">{entry.activity_id}</p>
        </div>
      </TableCell>
      <TableCell>
        <span className="text-sm">{entry.attempts}</span>
      </TableCell>
      <TableCell>
        <TooltipProvider>
          <Tooltip>
            <TooltipTrigger className="text-sm text-red-600 max-w-[200px] truncate block">
              {entry.last_error}
            </TooltipTrigger>
            <TooltipContent className="max-w-sm">
              <pre className="text-xs whitespace-pre-wrap">{entry.last_error}</pre>
            </TooltipContent>
          </Tooltip>
        </TooltipProvider>
      </TableCell>
      <TableCell>
        <TooltipProvider>
          <Tooltip>
            <TooltipTrigger className="text-sm text-muted-foreground">
              {formatDistanceToNow(new Date(entry.dead_at), { addSuffix: true })}
            </TooltipTrigger>
            <TooltipContent>
              {new Date(entry.dead_at).toLocaleString()}
            </TooltipContent>
          </Tooltip>
        </TooltipProvider>
      </TableCell>
      <TableCell>
        <span className="text-sm">{entry.requeue_count}</span>
      </TableCell>
      <TableCell>
        <div className="flex items-center gap-2">
          <Button variant="outline" size="sm" onClick={() => onRequeue(entry.id)}>
            <RotateCcw className="h-3 w-3 mr-1" />
            Requeue
          </Button>
          <Link href={`/durable/workflows/${entry.workflow_id}`}>
            <Button variant="ghost" size="sm">
              <ExternalLink className="h-3 w-3" />
            </Button>
          </Link>
        </div>
      </TableCell>
    </TableRow>
  );
}

export default function WorkflowsPage() {
  const [activeTab, setActiveTab] = useState<TabValue>("workflows");
  const [statusFilter, setStatusFilter] = useState<string>("all");
  const [searchQuery, setSearchQuery] = useState("");

  const workflowParams = statusFilter !== "all" ? { status: statusFilter } : undefined;
  const { data: workflowsData, isLoading: workflowsLoading, error: workflowsError, refetch: refetchWorkflows } = useWorkflows(workflowParams);
  const { data: tasksData, isLoading: tasksLoading, refetch: refetchTasks } = useTasks({ limit: 50 });
  const { data: dlqData, isLoading: dlqLoading, refetch: refetchDlq } = useDlq({ limit: 50 });
  const cancelMutation = useCancelWorkflow();
  const requeueMutation = useRequeueDlqEntry();

  const isLoading = workflowsLoading;

  if (isLoading) {
    return (
      <>
        <Header title="Workflows" />
        <div className="p-6 space-y-6">
          <Skeleton className="h-10 w-full" />
          <Skeleton className="h-96" />
        </div>
      </>
    );
  }

  if (workflowsError) {
    return (
      <>
        <Header title="Workflows" />
        <div className="p-6">
          <Card>
            <CardContent className="flex flex-col items-center justify-center py-12">
              <AlertTriangle className="h-12 w-12 text-muted-foreground mb-4" />
              <h3 className="text-lg font-medium mb-2">Unable to Load Workflows</h3>
              <p className="text-sm text-muted-foreground text-center max-w-md mb-4">
                The durable workflows API is not available.
              </p>
              <Button onClick={() => refetchWorkflows()} variant="outline">
                <RefreshCw className="h-4 w-4 mr-2" />
                Retry
              </Button>
            </CardContent>
          </Card>
        </div>
      </>
    );
  }

  const workflows = workflowsData?.data || [];
  const filteredWorkflows = searchQuery
    ? workflows.filter(w =>
        w.workflow_type.toLowerCase().includes(searchQuery.toLowerCase()) ||
        w.id.toLowerCase().includes(searchQuery.toLowerCase())
      )
    : workflows;

  const tasks = tasksData?.data || [];
  const dlqEntries = dlqData?.data || [];

  const handleCancel = (workflowId: string) => {
    if (confirm("Are you sure you want to cancel this workflow?")) {
      cancelMutation.mutate(workflowId);
    }
  };

  const handleRequeue = (dlqId: string) => {
    if (confirm("Are you sure you want to requeue this task?")) {
      requeueMutation.mutate(dlqId);
    }
  };

  return (
    <>
      <Header title="Workflows" />
      <div className="p-6 space-y-6">
        {/* Tab Navigation */}
        <div className="flex items-center gap-1 p-1 bg-muted rounded-lg w-fit">
          <button
            onClick={() => setActiveTab("workflows")}
            className={cn(
              "flex items-center gap-2 px-3 py-1.5 rounded-md text-sm font-medium transition-colors",
              activeTab === "workflows"
                ? "bg-background text-foreground shadow-sm"
                : "text-muted-foreground hover:text-foreground"
            )}
          >
            <Workflow className="h-4 w-4" />
            Workflows
            {workflows.length > 0 && (
              <Badge variant="secondary" className="ml-1">{workflowsData?.total || 0}</Badge>
            )}
          </button>
          <button
            onClick={() => setActiveTab("tasks")}
            className={cn(
              "flex items-center gap-2 px-3 py-1.5 rounded-md text-sm font-medium transition-colors",
              activeTab === "tasks"
                ? "bg-background text-foreground shadow-sm"
                : "text-muted-foreground hover:text-foreground"
            )}
          >
            <ListTodo className="h-4 w-4" />
            Tasks
            {tasks.length > 0 && (
              <Badge variant="secondary" className="ml-1">{tasksData?.total || 0}</Badge>
            )}
          </button>
          <button
            onClick={() => setActiveTab("dlq")}
            className={cn(
              "flex items-center gap-2 px-3 py-1.5 rounded-md text-sm font-medium transition-colors",
              activeTab === "dlq"
                ? "bg-background text-foreground shadow-sm"
                : "text-muted-foreground hover:text-foreground"
            )}
          >
            <Inbox className="h-4 w-4" />
            Dead Letter Queue
            {dlqEntries.length > 0 && (
              <Badge variant="destructive" className="ml-1">{dlqData?.total || 0}</Badge>
            )}
          </button>
        </div>

        {/* Workflows Tab Content */}
        {activeTab === "workflows" && (
          <div className="space-y-4">
            {/* Filters */}
            <div className="flex items-center gap-4">
              <div className="relative flex-1 max-w-sm">
                <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
                <Input
                  placeholder="Search workflows..."
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  className="pl-8"
                />
              </div>
              <Select value={statusFilter} onValueChange={setStatusFilter}>
                <SelectTrigger className="w-40">
                  <SelectValue placeholder="Status" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="all">All Status</SelectItem>
                  <SelectItem value="pending">Pending</SelectItem>
                  <SelectItem value="running">Running</SelectItem>
                  <SelectItem value="completed">Completed</SelectItem>
                  <SelectItem value="failed">Failed</SelectItem>
                  <SelectItem value="cancelled">Cancelled</SelectItem>
                </SelectContent>
              </Select>
              <Button variant="outline" size="sm" onClick={() => refetchWorkflows()}>
                <RefreshCw className="h-4 w-4 mr-2" />
                Refresh
              </Button>
            </div>

            {/* Workflows Table */}
            <Card>
              <CardContent className="pt-6">
                {filteredWorkflows.length > 0 ? (
                  <div className="rounded-md border">
                    <Table>
                      <TableHeader>
                        <TableRow>
                          <TableHead>Workflow</TableHead>
                          <TableHead>Status</TableHead>
                          <TableHead>Created</TableHead>
                          <TableHead>Started</TableHead>
                          <TableHead>Completed</TableHead>
                          <TableHead>Session</TableHead>
                          <TableHead>Actions</TableHead>
                        </TableRow>
                      </TableHeader>
                      <TableBody>
                        {filteredWorkflows.map((workflow) => (
                          <WorkflowRow
                            key={workflow.id}
                            workflow={workflow}
                            onCancel={handleCancel}
                          />
                        ))}
                      </TableBody>
                    </Table>
                  </div>
                ) : (
                  <div className="flex flex-col items-center justify-center py-12 text-muted-foreground">
                    <Workflow className="h-12 w-12 mb-4" />
                    <h3 className="text-lg font-medium mb-2">No Workflows Found</h3>
                    <p className="text-sm text-center max-w-md">
                      {searchQuery || statusFilter !== "all"
                        ? "No workflows match your search criteria."
                        : "No workflows have been created yet."}
                    </p>
                  </div>
                )}
              </CardContent>
            </Card>
          </div>
        )}

        {/* Tasks Tab Content */}
        {activeTab === "tasks" && (
          <div className="space-y-4">
            <div className="flex justify-end">
              <Button variant="outline" size="sm" onClick={() => refetchTasks()}>
                <RefreshCw className="h-4 w-4 mr-2" />
                Refresh
              </Button>
            </div>

            <Card>
              <CardHeader>
                <CardTitle>Task Queue</CardTitle>
                <CardDescription>Active and pending tasks in the queue</CardDescription>
              </CardHeader>
              <CardContent>
                {!tasksLoading && tasks.length > 0 ? (
                  <div className="rounded-md border">
                    <Table>
                      <TableHeader>
                        <TableRow>
                          <TableHead>Activity</TableHead>
                          <TableHead>Status</TableHead>
                          <TableHead>Priority</TableHead>
                          <TableHead>Attempt</TableHead>
                          <TableHead>Claimed By</TableHead>
                          <TableHead>Scheduled</TableHead>
                          <TableHead>Workflow</TableHead>
                        </TableRow>
                      </TableHeader>
                      <TableBody>
                        {tasks.map((task) => (
                          <TaskRow key={task.id} task={task} />
                        ))}
                      </TableBody>
                    </Table>
                  </div>
                ) : tasksLoading ? (
                  <Skeleton className="h-48" />
                ) : (
                  <div className="flex flex-col items-center justify-center py-12 text-muted-foreground">
                    <ListTodo className="h-12 w-12 mb-4" />
                    <h3 className="text-lg font-medium mb-2">No Tasks in Queue</h3>
                    <p className="text-sm text-center max-w-md">
                      The task queue is empty. Tasks will appear here when workflows schedule activities.
                    </p>
                  </div>
                )}
              </CardContent>
            </Card>
          </div>
        )}

        {/* DLQ Tab Content */}
        {activeTab === "dlq" && (
          <div className="space-y-4">
            <div className="flex justify-end">
              <Button variant="outline" size="sm" onClick={() => refetchDlq()}>
                <RefreshCw className="h-4 w-4 mr-2" />
                Refresh
              </Button>
            </div>

            <Card>
              <CardHeader>
                <CardTitle>Dead Letter Queue</CardTitle>
                <CardDescription>
                  Tasks that have exhausted all retry attempts
                </CardDescription>
              </CardHeader>
              <CardContent>
                {!dlqLoading && dlqEntries.length > 0 ? (
                  <div className="rounded-md border">
                    <Table>
                      <TableHeader>
                        <TableRow>
                          <TableHead>Activity</TableHead>
                          <TableHead>Attempts</TableHead>
                          <TableHead>Last Error</TableHead>
                          <TableHead>Dead At</TableHead>
                          <TableHead>Requeue Count</TableHead>
                          <TableHead>Actions</TableHead>
                        </TableRow>
                      </TableHeader>
                      <TableBody>
                        {dlqEntries.map((entry) => (
                          <DlqRow
                            key={entry.id}
                            entry={entry}
                            onRequeue={handleRequeue}
                          />
                        ))}
                      </TableBody>
                    </Table>
                  </div>
                ) : dlqLoading ? (
                  <Skeleton className="h-48" />
                ) : (
                  <div className="flex flex-col items-center justify-center py-12 text-muted-foreground">
                    <CheckCircle className="h-12 w-12 mb-4 text-green-500" />
                    <h3 className="text-lg font-medium mb-2">DLQ is Empty</h3>
                    <p className="text-sm text-center max-w-md">
                      No tasks have failed permanently. All tasks are being processed successfully.
                    </p>
                  </div>
                )}
              </CardContent>
            </Card>
          </div>
        )}
      </div>
    </>
  );
}
