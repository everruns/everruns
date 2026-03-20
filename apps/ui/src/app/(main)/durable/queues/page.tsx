"use client";

import { useState } from "react";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
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
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  useTasks,
  useTaskStats,
  useDlq,
  useRequeueDlqEntry,
  useDeleteDlqEntry,
  usePurgeDlq,
  useEnqueueTask,
} from "@/hooks";
import type { DurableTask, TaskStatus, DlqEntry, ActivityTypeStats } from "@/lib/api/types";
import {
  AlertTriangle,
  CheckCircle,
  Clock,
  XCircle,
  Activity,
  RefreshCw,
  Search,
  RotateCcw,
  Inbox,
  ListTodo,
  Plus,
  Trash2,
  ArrowUpDown,
  ExternalLink,
  Timer,
} from "lucide-react";
import Link from "next/link";
import { cn } from "@/lib/utils";
import { CopyButton } from "@/components/ui/copy-button";

type TabValue = "overview" | "tasks" | "dlq";

function formatDistanceToNow(date: Date, options?: { addSuffix?: boolean }): string {
  const now = new Date();
  const diff = now.getTime() - date.getTime();
  const future = diff < 0;
  const seconds = Math.floor(Math.abs(diff) / 1000);
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

  if (!options?.addSuffix) return result;
  return future ? `in ${result}` : `${result} ago`;
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)}ms`;
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`;
  return `${(ms / 60000).toFixed(1)}m`;
}

function getTaskStatusIcon(status: TaskStatus) {
  switch (status) {
    case "completed":
      return <CheckCircle className="h-4 w-4 text-green-500" />;
    case "claimed":
      return <Activity className="h-4 w-4 text-blue-500 animate-pulse" />;
    case "failed":
      return <XCircle className="h-4 w-4 text-red-500" />;
    case "dead":
      return <AlertTriangle className="h-4 w-4 text-red-700" />;
    case "cancelled":
      return <AlertTriangle className="h-4 w-4 text-yellow-500" />;
    default:
      return <Clock className="h-4 w-4 text-gray-500" />;
  }
}

function getTaskStatusBadgeVariant(status: TaskStatus) {
  switch (status) {
    case "completed":
      return "default" as const;
    case "claimed":
      return "secondary" as const;
    case "failed":
    case "dead":
      return "destructive" as const;
    default:
      return "outline" as const;
  }
}

// --- Queue Stats Card ---

function QueueStatsCard({
  activityType,
  stats,
}: {
  activityType: string;
  stats: ActivityTypeStats;
}) {
  const totalActive = stats.pending + stats.claimed;
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-sm font-medium truncate">{activityType}</CardTitle>
      </CardHeader>
      <CardContent>
        <div className="grid grid-cols-2 gap-3 text-sm">
          <div>
            <p className="text-muted-foreground text-xs">Pending</p>
            <p className="font-semibold text-lg">{stats.pending}</p>
          </div>
          <div>
            <p className="text-muted-foreground text-xs">Claimed</p>
            <p className="font-semibold text-lg">{stats.claimed}</p>
          </div>
          <div>
            <p className="text-muted-foreground text-xs">Completed/hr</p>
            <p className="font-medium">{stats.completed_last_hour}</p>
          </div>
          <div>
            <p className="text-muted-foreground text-xs">Failed/hr</p>
            <p className={cn("font-medium", stats.failed_last_hour > 0 && "text-red-500")}>
              {stats.failed_last_hour}
            </p>
          </div>
          <div>
            <p className="text-muted-foreground text-xs">Avg Duration</p>
            <p className="font-medium">{formatDuration(stats.avg_duration_ms)}</p>
          </div>
          <div>
            <p className="text-muted-foreground text-xs">p99 Duration</p>
            <p className="font-medium">{formatDuration(stats.p99_duration_ms)}</p>
          </div>
        </div>
        {totalActive > 0 && (
          <div className="mt-3">
            <div className="h-1.5 w-full bg-muted rounded-full overflow-hidden">
              <div
                className="h-full bg-blue-500 rounded-full"
                style={{ width: `${(stats.claimed / totalActive) * 100}%` }}
              />
            </div>
            <p className="text-xs text-muted-foreground mt-1">
              {stats.claimed}/{totalActive} processing
            </p>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

// --- Task Row ---

function TaskRow({ task }: { task: DurableTask }) {
  return (
    <TableRow>
      <TableCell>
        <div className="flex items-center gap-2">
          {getTaskStatusIcon(task.status)}
          <div>
            <p className="font-medium">{task.activity_type}</p>
            <p className="text-xs text-muted-foreground">{task.activity_id}</p>
          </div>
        </div>
      </TableCell>
      <TableCell>
        <Badge variant={getTaskStatusBadgeVariant(task.status)}>{task.status}</Badge>
      </TableCell>
      <TableCell>
        <Badge variant="outline">{task.priority}</Badge>
      </TableCell>
      <TableCell>
        <span className="text-sm">
          {task.attempt}/{task.max_attempts}
        </span>
      </TableCell>
      <TableCell>
        {task.claimed_by ? (
          <TooltipProvider>
            <Tooltip>
              <TooltipTrigger className="text-sm font-mono text-muted-foreground">
                {task.claimed_by.slice(0, 12)}...
              </TooltipTrigger>
              <TooltipContent>{task.claimed_by}</TooltipContent>
            </Tooltip>
          </TooltipProvider>
        ) : (
          <span className="text-sm text-muted-foreground">-</span>
        )}
      </TableCell>
      <TableCell>
        {task.last_error ? (
          <TooltipProvider>
            <Tooltip>
              <TooltipTrigger className="text-sm text-red-600 max-w-[150px] truncate block">
                {task.last_error}
              </TooltipTrigger>
              <TooltipContent className="max-w-sm">
                <pre className="text-xs whitespace-pre-wrap">{task.last_error}</pre>
              </TooltipContent>
            </Tooltip>
          </TooltipProvider>
        ) : (
          <span className="text-sm text-muted-foreground">-</span>
        )}
      </TableCell>
      <TableCell>
        <TooltipProvider>
          <Tooltip>
            <TooltipTrigger className="text-sm text-muted-foreground">
              {formatDistanceToNow(new Date(task.created_at), { addSuffix: true })}
            </TooltipTrigger>
            <TooltipContent>{new Date(task.created_at).toLocaleString()}</TooltipContent>
          </Tooltip>
        </TooltipProvider>
      </TableCell>
      <TableCell>
        <div className="flex items-center gap-1">
          <CopyButton value={task.id} />
          {task.workflow_id ? (
            <Link href={`/durable/workflows/${task.workflow_id}`}>
              <Button variant="ghost" size="sm">
                <ExternalLink className="h-3 w-3" />
              </Button>
            </Link>
          ) : (
            <Badge variant="outline" className="text-xs">
              standalone
            </Badge>
          )}
        </div>
      </TableCell>
    </TableRow>
  );
}

// --- DLQ Row ---

function DlqRow({
  entry,
  onRequeue,
  onDelete,
}: {
  entry: DlqEntry;
  onRequeue: (id: string) => void;
  onDelete: (id: string) => void;
}) {
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
              {entry.error_history.length > 1 && (
                <div className="mt-2 pt-2 border-t">
                  <p className="text-xs font-medium mb-1">
                    Error history ({entry.error_history.length}):
                  </p>
                  {entry.error_history.map((err, i) => (
                    <p key={i} className="text-xs text-muted-foreground">
                      {i + 1}. {err}
                    </p>
                  ))}
                </div>
              )}
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
            <TooltipContent>{new Date(entry.dead_at).toLocaleString()}</TooltipContent>
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
          <Button variant="ghost" size="sm" onClick={() => onDelete(entry.id)}>
            <Trash2 className="h-3 w-3 text-muted-foreground" />
          </Button>
          {entry.workflow_id ? (
            <Link href={`/durable/workflows/${entry.workflow_id}`}>
              <Button variant="ghost" size="sm">
                <ExternalLink className="h-3 w-3" />
              </Button>
            </Link>
          ) : (
            <Badge variant="outline" className="text-xs">
              standalone
            </Badge>
          )}
        </div>
      </TableCell>
    </TableRow>
  );
}

// --- Enqueue Dialog ---

function EnqueueDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [activityType, setActivityType] = useState("");
  const [inputJson, setInputJson] = useState("{}");
  const [maxAttempts, setMaxAttempts] = useState("3");
  const [priority, setPriority] = useState("0");
  const [jsonError, setJsonError] = useState<string | null>(null);
  const enqueueMutation = useEnqueueTask();

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    try {
      const parsed = JSON.parse(inputJson);
      setJsonError(null);
      enqueueMutation.mutate(
        {
          activity_type: activityType,
          input: parsed,
          max_attempts: parseInt(maxAttempts, 10),
          priority: parseInt(priority, 10),
        },
        {
          onSuccess: () => {
            setActivityType("");
            setInputJson("{}");
            setMaxAttempts("3");
            setPriority("0");
            onOpenChange(false);
          },
        },
      );
    } catch {
      setJsonError("Invalid JSON");
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Enqueue Task</DialogTitle>
          <DialogDescription>Add a standalone task to the queue.</DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="activity-type">Activity Type</Label>
            <Input
              id="activity-type"
              value={activityType}
              onChange={(e) => setActivityType(e.target.value)}
              placeholder="e.g. send_email"
              required
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="input-json">Input (JSON)</Label>
            <textarea
              id="input-json"
              value={inputJson}
              onChange={(e) => {
                setInputJson(e.target.value);
                setJsonError(null);
              }}
              className="flex min-h-[100px] w-full rounded-md border border-input bg-background px-3 py-2 text-sm font-mono ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              placeholder='{"key": "value"}'
            />
            {jsonError && <p className="text-sm text-destructive">{jsonError}</p>}
          </div>
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <Label htmlFor="max-attempts">Max Attempts</Label>
              <Input
                id="max-attempts"
                type="number"
                min="1"
                max="10"
                value={maxAttempts}
                onChange={(e) => setMaxAttempts(e.target.value)}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="priority">Priority</Label>
              <Input
                id="priority"
                type="number"
                min="-10"
                max="10"
                value={priority}
                onChange={(e) => setPriority(e.target.value)}
              />
            </div>
          </div>
          {enqueueMutation.isError && (
            <p className="text-sm text-destructive">
              Failed to enqueue: {enqueueMutation.error.message}
            </p>
          )}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={enqueueMutation.isPending || !activityType.trim()}>
              {enqueueMutation.isPending ? "Enqueuing..." : "Enqueue"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

// --- Main Page ---

export default function QueuesPage() {
  const [activeTab, setActiveTab] = useState<TabValue>("overview");
  const [statusFilter, setStatusFilter] = useState<string>("all");
  const [typeFilter, setTypeFilter] = useState<string>("all");
  const [searchQuery, setSearchQuery] = useState("");
  const [enqueueOpen, setEnqueueOpen] = useState(false);

  const taskParams = {
    ...(statusFilter !== "all" ? { status: statusFilter } : {}),
    ...(typeFilter !== "all" ? { activity_type: typeFilter } : {}),
    limit: 100,
  };
  const {
    data: tasksData,
    isLoading: tasksLoading,
    error: tasksError,
    refetch: refetchTasks,
  } = useTasks(taskParams);
  const { data: statsData, isLoading: statsLoading } = useTaskStats();
  const { data: dlqData, isLoading: dlqLoading, refetch: refetchDlq } = useDlq({ limit: 100 });
  const requeueMutation = useRequeueDlqEntry();
  const deleteDlqMutation = useDeleteDlqEntry();
  const purgeDlqMutation = usePurgeDlq();

  const tasks = tasksData?.data || [];
  const dlqEntries = dlqData?.data || [];
  const stats = statsData;

  // Derive activity types from stats for the type filter
  const activityTypes = stats ? Object.keys(stats.by_activity_type).sort() : [];

  // Filter tasks by search
  const filteredTasks = searchQuery
    ? tasks.filter(
        (t) =>
          t.activity_type.toLowerCase().includes(searchQuery.toLowerCase()) ||
          t.activity_id.toLowerCase().includes(searchQuery.toLowerCase()) ||
          t.id.toLowerCase().includes(searchQuery.toLowerCase()),
      )
    : tasks;

  // Aggregate stats
  const totalPending = stats
    ? Object.values(stats.by_activity_type).reduce((sum, s) => sum + s.pending, 0)
    : 0;
  const totalClaimed = stats
    ? Object.values(stats.by_activity_type).reduce((sum, s) => sum + s.claimed, 0)
    : 0;
  const totalCompletedHr = stats
    ? Object.values(stats.by_activity_type).reduce((sum, s) => sum + s.completed_last_hour, 0)
    : 0;
  const totalFailedHr = stats
    ? Object.values(stats.by_activity_type).reduce((sum, s) => sum + s.failed_last_hour, 0)
    : 0;

  const handleRequeue = (dlqId: string) => {
    if (confirm("Requeue this task?")) {
      requeueMutation.mutate(dlqId);
    }
  };

  const handleDeleteDlq = (dlqId: string) => {
    if (confirm("Delete this DLQ entry permanently?")) {
      deleteDlqMutation.mutate(dlqId);
    }
  };

  const handlePurgeDlq = () => {
    if (confirm("Purge ALL dead letter queue entries? This cannot be undone.")) {
      purgeDlqMutation.mutate();
    }
  };

  if (tasksLoading && statsLoading) {
    return (
      <div className="container mx-auto p-6">
        <div className="flex items-center justify-between mb-6">
          <h1 className="text-2xl font-bold">Queues</h1>
        </div>
        <div className="space-y-6">
          <Skeleton className="h-10 w-full" />
          <div className="grid grid-cols-1 md:grid-cols-4 gap-4">
            {[...Array(4)].map((_, i) => (
              <Skeleton key={i} className="h-24" />
            ))}
          </div>
          <Skeleton className="h-96" />
        </div>
      </div>
    );
  }

  if (tasksError) {
    return (
      <div className="container mx-auto p-6">
        <div className="flex items-center justify-between mb-6">
          <h1 className="text-2xl font-bold">Queues</h1>
        </div>
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-12">
            <AlertTriangle className="h-12 w-12 text-muted-foreground mb-4" />
            <h3 className="text-lg font-medium mb-2">Unable to Load Queues</h3>
            <p className="text-sm text-muted-foreground text-center max-w-md mb-4">
              The durable queues API is not available.
            </p>
            <Button onClick={() => refetchTasks()} variant="outline">
              <RefreshCw className="h-4 w-4 mr-2" />
              Retry
            </Button>
          </CardContent>
        </Card>
      </div>
    );
  }

  return (
    <div className="container mx-auto p-6">
      <div className="flex items-center justify-between mb-6">
        <h1 className="text-2xl font-bold">Queues</h1>
        <Button onClick={() => setEnqueueOpen(true)}>
          <Plus className="h-4 w-4 mr-2" />
          Enqueue Task
        </Button>
      </div>

      {/* Summary Stats */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-4 mb-6">
        <Card>
          <CardContent className="pt-4 pb-4">
            <div className="flex items-center gap-2">
              <Clock className="h-4 w-4 text-muted-foreground" />
              <p className="text-sm text-muted-foreground">Pending</p>
            </div>
            <p className="text-2xl font-bold mt-1">{totalPending}</p>
          </CardContent>
        </Card>
        <Card>
          <CardContent className="pt-4 pb-4">
            <div className="flex items-center gap-2">
              <Activity className="h-4 w-4 text-blue-500" />
              <p className="text-sm text-muted-foreground">Processing</p>
            </div>
            <p className="text-2xl font-bold mt-1">{totalClaimed}</p>
          </CardContent>
        </Card>
        <Card>
          <CardContent className="pt-4 pb-4">
            <div className="flex items-center gap-2">
              <CheckCircle className="h-4 w-4 text-green-500" />
              <p className="text-sm text-muted-foreground">Completed/hr</p>
            </div>
            <p className="text-2xl font-bold mt-1">{totalCompletedHr}</p>
          </CardContent>
        </Card>
        <Card>
          <CardContent className="pt-4 pb-4">
            <div className="flex items-center gap-2">
              <XCircle className="h-4 w-4 text-red-500" />
              <p className="text-sm text-muted-foreground">Failed/hr</p>
            </div>
            <p className={cn("text-2xl font-bold mt-1", totalFailedHr > 0 && "text-red-500")}>
              {totalFailedHr}
            </p>
          </CardContent>
        </Card>
      </div>

      {/* Global latency stats */}
      {stats && (
        <div className="flex items-center gap-6 text-sm text-muted-foreground mb-6">
          <div className="flex items-center gap-1">
            <Timer className="h-3.5 w-3.5" />
            <span>Oldest pending: {formatDuration(stats.oldest_pending_task_age_ms)}</span>
          </div>
          <div className="flex items-center gap-1">
            <ArrowUpDown className="h-3.5 w-3.5" />
            <span>Avg wait: {formatDuration(stats.avg_schedule_to_start_ms)}</span>
          </div>
          <div className="flex items-center gap-1">
            <Activity className="h-3.5 w-3.5" />
            <span>Avg exec: {formatDuration(stats.avg_execution_time_ms)}</span>
          </div>
        </div>
      )}

      {/* Tab Navigation */}
      <div className="space-y-6">
        <div className="flex items-center gap-1 p-1 bg-muted rounded-lg w-fit">
          <button
            onClick={() => setActiveTab("overview")}
            className={cn(
              "flex items-center gap-2 px-3 py-1.5 rounded-md text-sm font-medium transition-colors",
              activeTab === "overview"
                ? "bg-background text-foreground shadow-sm"
                : "text-muted-foreground hover:text-foreground",
            )}
          >
            <ArrowUpDown className="h-4 w-4" />
            By Activity Type
            {activityTypes.length > 0 && (
              <Badge variant="secondary" className="ml-1">
                {activityTypes.length}
              </Badge>
            )}
          </button>
          <button
            onClick={() => setActiveTab("tasks")}
            className={cn(
              "flex items-center gap-2 px-3 py-1.5 rounded-md text-sm font-medium transition-colors",
              activeTab === "tasks"
                ? "bg-background text-foreground shadow-sm"
                : "text-muted-foreground hover:text-foreground",
            )}
          >
            <ListTodo className="h-4 w-4" />
            Tasks
            {tasksData?.total !== undefined && (
              <Badge variant="secondary" className="ml-1">
                {tasksData.total}
              </Badge>
            )}
          </button>
          <button
            onClick={() => setActiveTab("dlq")}
            className={cn(
              "flex items-center gap-2 px-3 py-1.5 rounded-md text-sm font-medium transition-colors",
              activeTab === "dlq"
                ? "bg-background text-foreground shadow-sm"
                : "text-muted-foreground hover:text-foreground",
            )}
          >
            <Inbox className="h-4 w-4" />
            Dead Letter Queue
            {dlqEntries.length > 0 && (
              <Badge variant="destructive" className="ml-1">
                {dlqData?.total || 0}
              </Badge>
            )}
          </button>
        </div>

        {/* Overview Tab - Stats by Activity Type */}
        {activeTab === "overview" && (
          <div className="space-y-4">
            {statsLoading ? (
              <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
                {[...Array(3)].map((_, i) => (
                  <Skeleton key={i} className="h-48" />
                ))}
              </div>
            ) : activityTypes.length > 0 ? (
              <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
                {activityTypes.map((type) => (
                  <QueueStatsCard
                    key={type}
                    activityType={type}
                    stats={stats!.by_activity_type[type]}
                  />
                ))}
              </div>
            ) : (
              <Card>
                <CardContent className="flex flex-col items-center justify-center py-12 text-muted-foreground">
                  <ListTodo className="h-12 w-12 mb-4" />
                  <h3 className="text-lg font-medium mb-2">No Queue Activity</h3>
                  <p className="text-sm text-center max-w-md">
                    No activity types have been registered yet. Enqueue a task or start a workflow
                    to see queue statistics.
                  </p>
                </CardContent>
              </Card>
            )}

            {/* Priority Distribution */}
            {stats && Object.keys(stats.by_priority).length > 0 && (
              <Card>
                <CardHeader>
                  <CardTitle className="text-sm">Priority Distribution</CardTitle>
                  <CardDescription>Pending tasks grouped by priority level</CardDescription>
                </CardHeader>
                <CardContent>
                  <div className="flex items-end gap-2 h-24">
                    {Object.entries(stats.by_priority)
                      .sort(([a], [b]) => parseInt(b) - parseInt(a))
                      .map(([priority, count]) => {
                        const maxCount = Math.max(...Object.values(stats.by_priority));
                        const height = maxCount > 0 ? (count / maxCount) * 100 : 0;
                        return (
                          <TooltipProvider key={priority}>
                            <Tooltip>
                              <TooltipTrigger className="flex flex-col items-center gap-1 flex-1">
                                <div
                                  className="w-full bg-primary/20 rounded-t min-h-[4px]"
                                  style={{ height: `${height}%` }}
                                />
                                <span className="text-xs text-muted-foreground">{priority}</span>
                              </TooltipTrigger>
                              <TooltipContent>
                                Priority {priority}: {count} task{count !== 1 ? "s" : ""}
                              </TooltipContent>
                            </Tooltip>
                          </TooltipProvider>
                        );
                      })}
                  </div>
                </CardContent>
              </Card>
            )}
          </div>
        )}

        {/* Tasks Tab */}
        {activeTab === "tasks" && (
          <div className="space-y-4">
            {/* Filters */}
            <div className="flex items-center gap-4">
              <div className="relative flex-1 max-w-sm">
                <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
                <Input
                  placeholder="Search tasks..."
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
                  <SelectItem value="claimed">Claimed</SelectItem>
                  <SelectItem value="completed">Completed</SelectItem>
                  <SelectItem value="failed">Failed</SelectItem>
                  <SelectItem value="cancelled">Cancelled</SelectItem>
                </SelectContent>
              </Select>
              {activityTypes.length > 0 && (
                <Select value={typeFilter} onValueChange={setTypeFilter}>
                  <SelectTrigger className="w-48">
                    <SelectValue placeholder="Activity Type" />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">All Types</SelectItem>
                    {activityTypes.map((type) => (
                      <SelectItem key={type} value={type}>
                        {type}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              )}
              <Button variant="outline" size="sm" onClick={() => refetchTasks()}>
                <RefreshCw className="h-4 w-4 mr-2" />
                Refresh
              </Button>
            </div>

            {/* Tasks Table */}
            <Card>
              <CardContent className="pt-6">
                {tasksLoading ? (
                  <Skeleton className="h-48" />
                ) : filteredTasks.length > 0 ? (
                  <div className="rounded-md border">
                    <Table>
                      <TableHeader>
                        <TableRow>
                          <TableHead>Activity</TableHead>
                          <TableHead>Status</TableHead>
                          <TableHead>Priority</TableHead>
                          <TableHead>Attempt</TableHead>
                          <TableHead>Claimed By</TableHead>
                          <TableHead>Error</TableHead>
                          <TableHead>Created</TableHead>
                          <TableHead>Actions</TableHead>
                        </TableRow>
                      </TableHeader>
                      <TableBody>
                        {filteredTasks.map((task) => (
                          <TaskRow key={task.id} task={task} />
                        ))}
                      </TableBody>
                    </Table>
                  </div>
                ) : (
                  <div className="flex flex-col items-center justify-center py-12 text-muted-foreground">
                    <ListTodo className="h-12 w-12 mb-4" />
                    <h3 className="text-lg font-medium mb-2">No Tasks Found</h3>
                    <p className="text-sm text-center max-w-md">
                      {searchQuery || statusFilter !== "all" || typeFilter !== "all"
                        ? "No tasks match your filters."
                        : "The task queue is empty."}
                    </p>
                  </div>
                )}
              </CardContent>
            </Card>
          </div>
        )}

        {/* DLQ Tab */}
        {activeTab === "dlq" && (
          <div className="space-y-4">
            <div className="flex justify-end gap-2">
              {dlqEntries.length > 0 && (
                <Button
                  variant="outline"
                  size="sm"
                  onClick={handlePurgeDlq}
                  disabled={purgeDlqMutation.isPending}
                  className="text-destructive hover:text-destructive"
                >
                  <Trash2 className="h-4 w-4 mr-2" />
                  {purgeDlqMutation.isPending ? "Purging..." : "Purge All"}
                </Button>
              )}
              <Button variant="outline" size="sm" onClick={() => refetchDlq()}>
                <RefreshCw className="h-4 w-4 mr-2" />
                Refresh
              </Button>
            </div>

            <Card>
              <CardHeader>
                <CardTitle>Dead Letter Queue</CardTitle>
                <CardDescription>
                  Tasks that exhausted all retry attempts. Requeue to retry or delete to discard.
                </CardDescription>
              </CardHeader>
              <CardContent>
                {dlqLoading ? (
                  <Skeleton className="h-48" />
                ) : dlqEntries.length > 0 ? (
                  <div className="rounded-md border">
                    <Table>
                      <TableHeader>
                        <TableRow>
                          <TableHead>Activity</TableHead>
                          <TableHead>Attempts</TableHead>
                          <TableHead>Last Error</TableHead>
                          <TableHead>Dead At</TableHead>
                          <TableHead>Requeues</TableHead>
                          <TableHead>Actions</TableHead>
                        </TableRow>
                      </TableHeader>
                      <TableBody>
                        {dlqEntries.map((entry) => (
                          <DlqRow
                            key={entry.id}
                            entry={entry}
                            onRequeue={handleRequeue}
                            onDelete={handleDeleteDlq}
                          />
                        ))}
                      </TableBody>
                    </Table>
                  </div>
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

      <EnqueueDialog open={enqueueOpen} onOpenChange={setEnqueueOpen} />
    </div>
  );
}
