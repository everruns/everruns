"use client";
import { useConfirmation } from "@/components/ui/confirmation-provider";
import { PageContainer, PageMasthead } from "@/components/layout";

import { useUiTranslations } from "@/providers/locale-provider";
import { useDurablePageRows } from "@/components/chat-mode/use-durable-page-rows";

import { useChatPageFilters } from "@/components/chat-mode/page-view-state";

import { useState } from "react";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { SearchInput } from "@/components/ui/search-input";
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
  usePageTitle,
} from "@/hooks";
import {
  AlertTriangle,
  CheckCircle,
  Clock,
  XCircle,
  Activity,
  RefreshCw,
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
  const requestConfirmation = useConfirmation();
  const ui = useUiTranslations();
  usePageTitle(ui("Queues"), ui("Durable"));
  const [activeTab, setActiveTab] = useState<TabValue>("overview");
  const [statusFilter, setStatusFilter] = useState<string>("all");
  const [typeFilter, setTypeFilter] = useState<string>("all");
  const [searchQuery, setSearchQuery] = useState("");
  useChatPageFilters({
    tab: activeTab,
    status: statusFilter,
    type: typeFilter,
    search: searchQuery,
  });
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

  useDurablePageRows(
    "Queue tasks",
    filteredTasks.map((t) => ({
      id: t.id,
      activity_type: t.activity_type,
      status: t.status,
      priority: t.priority,
      attempt: t.attempt,
      max_attempts: t.max_attempts,
    })),
    { enabled: activeTab === "tasks", loading: tasksLoading, error: !!tasksError },
  );
  useDurablePageRows(
    "Queue overview",
    Object.entries(stats?.by_activity_type ?? {}).map(([type, s]) => ({
      activity_type: type,
      pending: s.pending,
      claimed: s.claimed,
      completed_last_hour: s.completed_last_hour,
      failed_last_hour: s.failed_last_hour,
    })),
    { enabled: activeTab === "overview", loading: statsLoading },
  );
  useDurablePageRows(
    "Dead letter queue",
    dlqEntries.map((t) => ({
      id: t.id,
      activity_type: t.activity_type,
      attempts: t.attempts,
      dead_at: t.dead_at,
      requeue_count: t.requeue_count,
    })),
    { enabled: activeTab === "dlq", loading: dlqLoading },
  );

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

  const handleRequeue = async (dlqId: string) => {
    if (await requestConfirmation("Requeue this task?")) {
      requeueMutation.mutate(dlqId);
    }
  };

  const handleDeleteDlq = async (dlqId: string) => {
    if (await requestConfirmation("Delete this DLQ entry permanently?")) {
      deleteDlqMutation.mutate(dlqId);
    }
  };

  const handlePurgeDlq = async () => {
    if (await requestConfirmation("Purge ALL dead letter queue entries? This cannot be undone.")) {
      purgeDlqMutation.mutate();
    }
  };

  if (tasksLoading && statsLoading) {
    return (
      <PageContainer className="operational-page">
        <div className="operational-toolbar flex flex-wrap items-center justify-between gap-3 mb-6">
          <PageMasthead title={ui("Queues")} className="min-w-0 flex-1" />
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
      </PageContainer>
    );
  }

  if (tasksError) {
    return (
      <PageContainer className="operational-page">
        <div className="operational-toolbar flex flex-wrap items-center justify-between gap-3 mb-6">
          <PageMasthead title={ui("Queues")} className="min-w-0 flex-1" />
        </div>
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-12">
            <AlertTriangle className="h-12 w-12 text-muted-foreground mb-4" />
            <h3 className="text-lg font-medium mb-2">{ui("Unable to Load Queues")}</h3>
            <p className="text-sm text-muted-foreground text-center max-w-md mb-4">
              {ui("The durable queues API is not available.")}
            </p>
            <Button onClick={() => refetchTasks()} variant="outline">
              <RefreshCw className="h-4 w-4 mr-2" />
              {ui("Retry")}
            </Button>
          </CardContent>
        </Card>
      </PageContainer>
    );
  }

  return (
    <PageContainer className="operational-page">
      <div className="operational-toolbar flex flex-wrap items-center justify-between gap-3 mb-6">
        <PageMasthead title={ui("Queues")} className="min-w-0 flex-1" />
        <Button onClick={() => setEnqueueOpen(true)}>
          <Plus className="h-4 w-4 mr-2" />
          {ui("Enqueue Task")}
        </Button>
      </div>

      {/* Summary Stats */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-4 mb-6">
        <Card>
          <CardContent className="pt-4 pb-4">
            <div className="flex items-center gap-2">
              <Clock className="h-4 w-4 text-muted-foreground" />
              <p className="text-sm text-muted-foreground">{ui("Pending")}</p>
            </div>
            <p className="text-2xl font-bold mt-1">{totalPending}</p>
          </CardContent>
        </Card>
        <Card>
          <CardContent className="pt-4 pb-4">
            <div className="flex items-center gap-2">
              <Activity className="h-4 w-4 text-info" />
              <p className="text-sm text-muted-foreground">{ui("Processing")}</p>
            </div>
            <p className="text-2xl font-bold mt-1">{totalClaimed}</p>
          </CardContent>
        </Card>
        <Card>
          <CardContent className="pt-4 pb-4">
            <div className="flex items-center gap-2">
              <CheckCircle className="h-4 w-4 text-success" />
              <p className="text-sm text-muted-foreground">{ui("Completed/hr")}</p>
            </div>
            <p className="text-2xl font-bold mt-1">{totalCompletedHr}</p>
          </CardContent>
        </Card>
        <Card>
          <CardContent className="pt-4 pb-4">
            <div className="flex items-center gap-2">
              <XCircle className="h-4 w-4 text-destructive" />
              <p className="text-sm text-muted-foreground">{ui("Failed/hr")}</p>
            </div>
            <p className={cn("text-2xl font-bold mt-1", totalFailedHr > 0 && "text-destructive")}>
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
            <span>
              {ui("Oldest pending:")} {formatDurationCompact(stats.oldest_pending_task_age_ms)}
            </span>
          </div>
          <div className="flex items-center gap-1">
            <ArrowUpDown className="h-3.5 w-3.5" />
            <span>
              {ui("Avg wait:")} {formatDurationCompact(stats.avg_schedule_to_start_ms)}
            </span>
          </div>
          <div className="flex items-center gap-1">
            <Activity className="h-3.5 w-3.5" />
            <span>
              {ui("Avg exec:")} {formatDurationCompact(stats.avg_execution_time_ms)}
            </span>
          </div>
        </div>
      )}

      {/* Tab Navigation */}
      <div className="space-y-6">
        <div className="flex items-center gap-1 p-1 bg-muted w-fit">
          <button
            onClick={() => setActiveTab("overview")}
            className={cn(
              "flex items-center gap-2 px-3 py-1.5 text-sm font-medium transition-colors",
              activeTab === "overview"
                ? "bg-background text-foreground shadow-sm"
                : "text-muted-foreground hover:text-foreground",
            )}
          >
            <ArrowUpDown className="h-4 w-4" />
            {ui("By Activity Type")}
            {activityTypes.length > 0 && (
              <Badge variant="secondary" className="ml-1">
                {activityTypes.length}
              </Badge>
            )}
          </button>
          <button
            onClick={() => setActiveTab("tasks")}
            className={cn(
              "flex items-center gap-2 px-3 py-1.5 text-sm font-medium transition-colors",
              activeTab === "tasks"
                ? "bg-background text-foreground shadow-sm"
                : "text-muted-foreground hover:text-foreground",
            )}
          >
            <ListTodo className="h-4 w-4" />
            {ui("Tasks")}
            {tasksData?.total !== undefined && (
              <Badge variant="secondary" className="ml-1">
                {tasksData.total}
              </Badge>
            )}
          </button>
          <button
            onClick={() => setActiveTab("dlq")}
            className={cn(
              "flex items-center gap-2 px-3 py-1.5 text-sm font-medium transition-colors",
              activeTab === "dlq"
                ? "bg-background text-foreground shadow-sm"
                : "text-muted-foreground hover:text-foreground",
            )}
          >
            <Inbox className="h-4 w-4" />
            {ui("Dead Letter Queue")}
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
                  <h3 className="text-lg font-medium mb-2">{ui("No Queue Activity")}</h3>
                  <p className="text-sm text-center max-w-md">
                    {ui(
                      "No activity types have been registered yet. Enqueue a task or start a workflow to see queue statistics.",
                    )}
                  </p>
                </CardContent>
              </Card>
            )}

            {/* Priority Distribution */}
            {stats && Object.keys(stats.by_priority).length > 0 && (
              <Card>
                <CardHeader>
                  <CardTitle className="text-sm">{ui("Priority Distribution")}</CardTitle>
                  <CardDescription>{ui("Pending tasks grouped by priority level")}</CardDescription>
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
                                  className="w-full bg-primary/20 min-h-[4px]"
                                  style={{ height: `${height}%` }}
                                />
                                <span className="text-xs text-muted-foreground">{priority}</span>
                              </TooltipTrigger>
                              <TooltipContent>
                                {ui("Priority")} {priority}: {count} {ui("task")}
                                {count !== 1 ? "s" : ""}
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
              <SearchInput
                containerClassName="flex-1 max-w-sm"
                placeholder={ui("Search tasks...")}
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
              />
              <Select value={statusFilter} onValueChange={setStatusFilter}>
                <SelectTrigger className="w-40">
                  <SelectValue placeholder={ui("Status")} />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="all">{ui("All Status")}</SelectItem>
                  <SelectItem value="pending">{ui("Pending")}</SelectItem>
                  <SelectItem value="claimed">{ui("Claimed")}</SelectItem>
                  <SelectItem value="completed">{ui("Completed")}</SelectItem>
                  <SelectItem value="failed">{ui("Failed")}</SelectItem>
                  <SelectItem value="cancelled">{ui("Cancelled")}</SelectItem>
                </SelectContent>
              </Select>
              {activityTypes.length > 0 && (
                <Select value={typeFilter} onValueChange={setTypeFilter}>
                  <SelectTrigger className="w-48">
                    <SelectValue placeholder={ui("Activity Type")} />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">{ui("All Types")}</SelectItem>
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
                {ui("Refresh")}
              </Button>
            </div>

            {/* Tasks Table */}
            <Card>
              <CardContent className="pt-6">
                {tasksLoading ? (
                  <Skeleton className="h-48" />
                ) : filteredTasks.length > 0 ? (
                  <div className="border">
                    <Table>
                      <TableHeader>
                        <TableRow>
                          <TableHead>{ui("Activity")}</TableHead>
                          <TableHead>{ui("Status")}</TableHead>
                          <TableHead>{ui("Priority")}</TableHead>
                          <TableHead>{ui("Attempt")}</TableHead>
                          <TableHead>{ui("Claimed By")}</TableHead>
                          <TableHead>{ui("Error")}</TableHead>
                          <TableHead>{ui("Created")}</TableHead>
                          <TableHead>{ui("Actions")}</TableHead>
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
                    <h3 className="text-lg font-medium mb-2">{ui("No Tasks Found")}</h3>
                    <p className="text-sm text-center max-w-md">
                      {searchQuery || statusFilter !== "all" || typeFilter !== "all"
                        ? ui("No tasks match your filters.")
                        : ui("The task queue is empty.")}
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
                  {purgeDlqMutation.isPending ? ui("Purging...") : ui("Purge All")}
                </Button>
              )}
              <Button variant="outline" size="sm" onClick={() => refetchDlq()}>
                <RefreshCw className="h-4 w-4 mr-2" />
                {ui("Refresh")}
              </Button>
            </div>

            <Card>
              <CardHeader>
                <CardTitle>{ui("Dead Letter Queue")}</CardTitle>
                <CardDescription>
                  {ui(
                    "Tasks that exhausted all retry attempts. Requeue to retry or delete to discard.",
                  )}
                </CardDescription>
              </CardHeader>
              <CardContent>
                {dlqLoading ? (
                  <Skeleton className="h-48" />
                ) : dlqEntries.length > 0 ? (
                  <div className="border">
                    <Table>
                      <TableHeader>
                        <TableRow>
                          <TableHead>{ui("Activity")}</TableHead>
                          <TableHead>{ui("Attempts")}</TableHead>
                          <TableHead>{ui("Last Error")}</TableHead>
                          <TableHead>{ui("Dead At")}</TableHead>
                          <TableHead>{ui("Requeues")}</TableHead>
                          <TableHead>{ui("Actions")}</TableHead>
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
                    <CheckCircle className="h-12 w-12 mb-4 text-success" />
                    <h3 className="text-lg font-medium mb-2">{ui("DLQ is Empty")}</h3>
                    <p className="text-sm text-center max-w-md">
                      {ui(
                        "No tasks have failed permanently. All tasks are being processed successfully.",
                      )}
                    </p>
                  </div>
                )}
              </CardContent>
            </Card>
          </div>
        )}
      </div>

      <EnqueueDialog open={enqueueOpen} onOpenChange={setEnqueueOpen} />
    </PageContainer>
  );
}
