"use client";

import { useState } from "react";
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
import { Table, TableBody, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import {
  useTasks,
  useTaskStats,
  useDlq,
  useRequeueDlqEntry,
  useDeleteDlqEntry,
  usePurgeDlq,
} from "@/hooks";
import {
  AlertTriangle,
  CheckCircle,
  Clock,
  XCircle,
  Activity,
  RefreshCw,
  Search,
  Inbox,
  ListTodo,
  Plus,
  Trash2,
  ArrowUpDown,
  Timer,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { formatDurationCompact } from "@/lib/formatting";

import { QueueStatsCard } from "./queue-stats-card";
import { TaskRow } from "./task-row";
import { DlqRow } from "./dlq-row";
import { EnqueueDialog } from "./enqueue-dialog";

type TabValue = "overview" | "tasks" | "dlq";

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
            <span>Oldest pending: {formatDurationCompact(stats.oldest_pending_task_age_ms)}</span>
          </div>
          <div className="flex items-center gap-1">
            <ArrowUpDown className="h-3.5 w-3.5" />
            <span>Avg wait: {formatDurationCompact(stats.avg_schedule_to_start_ms)}</span>
          </div>
          <div className="flex items-center gap-1">
            <Activity className="h-3.5 w-3.5" />
            <span>Avg exec: {formatDurationCompact(stats.avg_execution_time_ms)}</span>
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
