"use client";

import { useState } from "react";
import { useParams, useRouter } from "next/navigation";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
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
  DialogTrigger,
} from "@/components/ui/dialog";
import {
  useSchedule,
  useScheduleExecutions,
  useScheduleStats,
  useUpdateSchedule,
  useDeleteSchedule,
  usePauseSchedule,
  useResumeSchedule,
  useTriggerSchedule,
} from "@/hooks";
import type {
  ScheduleExecution,
  ScheduleExecutionStatus,
  UpdateScheduleRequest,
} from "@/lib/api/types";
import {
  Clock,
  AlertTriangle,
  CheckCircle,
  XCircle,
  Activity,
  RefreshCw,
  Play,
  Pause,
  Trash2,
  Zap,
  ArrowLeft,
  Settings,
  BarChart3,
  History,
  ExternalLink,
  SkipForward,
} from "lucide-react";
import Link from "next/link";
import { CopyButton } from "@/components/ui/copy-button";
import { formatDistanceToNow } from "@/lib/formatting";
import { getScheduleExecutionBadgeVariant } from "@/lib/status-utils";

function getStatusIcon(status: ScheduleExecutionStatus) {
  switch (status) {
    case "completed":
      return <CheckCircle className="h-4 w-4 text-green-500" />;
    case "running":
      return <Activity className="h-4 w-4 text-blue-500 animate-pulse" />;
    case "failed":
      return <XCircle className="h-4 w-4 text-red-500" />;
    case "skipped":
      return <SkipForward className="h-4 w-4 text-yellow-500" />;
    case "pending":
    default:
      return <Clock className="h-4 w-4 text-gray-500" />;
  }
}

function ExecutionRow({ execution }: { execution: ScheduleExecution }) {
  return (
    <TableRow>
      <TableCell>
        <div className="flex items-center gap-2">
          {getStatusIcon(execution.status)}
          <Badge variant={getScheduleExecutionBadgeVariant(execution.status)}>
            {execution.status}
          </Badge>
        </div>
      </TableCell>
      <TableCell>
        <TooltipProvider>
          <Tooltip>
            <TooltipTrigger className="text-sm text-muted-foreground">
              {formatDistanceToNow(new Date(execution.scheduled_at), { addSuffix: true })}
            </TooltipTrigger>
            <TooltipContent>{new Date(execution.scheduled_at).toLocaleString()}</TooltipContent>
          </Tooltip>
        </TooltipProvider>
      </TableCell>
      <TableCell>
        <TooltipProvider>
          <Tooltip>
            <TooltipTrigger className="text-sm text-muted-foreground">
              {formatDistanceToNow(new Date(execution.started_at), { addSuffix: true })}
            </TooltipTrigger>
            <TooltipContent>{new Date(execution.started_at).toLocaleString()}</TooltipContent>
          </Tooltip>
        </TooltipProvider>
      </TableCell>
      <TableCell>
        {execution.completed_at ? (
          <TooltipProvider>
            <Tooltip>
              <TooltipTrigger className="text-sm text-muted-foreground">
                {formatDistanceToNow(new Date(execution.completed_at), { addSuffix: true })}
              </TooltipTrigger>
              <TooltipContent>{new Date(execution.completed_at).toLocaleString()}</TooltipContent>
            </Tooltip>
          </TooltipProvider>
        ) : (
          <span className="text-sm text-muted-foreground">-</span>
        )}
      </TableCell>
      <TableCell>
        {execution.duration_ms !== undefined ? (
          <span className="text-sm">{execution.duration_ms}ms</span>
        ) : (
          <span className="text-sm text-muted-foreground">-</span>
        )}
      </TableCell>
      <TableCell>
        {execution.workflow_id ? (
          <Link
            href={`/durable/workflows/${execution.workflow_id}`}
            className="flex items-center gap-1 text-sm text-primary hover:underline"
          >
            <ExternalLink className="h-3 w-3" />
            View Workflow
          </Link>
        ) : (
          <span className="text-sm text-muted-foreground">-</span>
        )}
      </TableCell>
      <TableCell>
        {execution.error ? (
          <TooltipProvider>
            <Tooltip>
              <TooltipTrigger className="text-sm text-red-500 max-w-[200px] truncate block">
                {execution.error}
              </TooltipTrigger>
              <TooltipContent className="max-w-sm">
                <pre className="text-xs whitespace-pre-wrap">{execution.error}</pre>
              </TooltipContent>
            </Tooltip>
          </TooltipProvider>
        ) : (
          <span className="text-sm text-muted-foreground">-</span>
        )}
      </TableCell>
    </TableRow>
  );
}

function EditScheduleDialog({
  schedule,
  onClose,
}: {
  schedule: {
    id: string;
    description?: string;
    cron_expression: string;
    timezone: string;
    target: { type: string; name: string; input: Record<string, unknown> };
    enabled: boolean;
    max_concurrent?: number;
    catch_up_missed: boolean;
    max_catch_up?: number;
  };
  onClose: () => void;
}) {
  const [description, setDescription] = useState(schedule.description || "");
  const [cronExpression, setCronExpression] = useState(schedule.cron_expression);
  const [maxConcurrent, setMaxConcurrent] = useState(schedule.max_concurrent?.toString() || "");
  const [catchUpMissed, setCatchUpMissed] = useState(schedule.catch_up_missed);
  const [maxCatchUp, setMaxCatchUp] = useState(schedule.max_catch_up?.toString() || "");

  const updateMutation = useUpdateSchedule();

  const handleSubmit = async () => {
    const request: UpdateScheduleRequest = {};

    if (description !== (schedule.description || "")) {
      request.description = description || undefined;
    }
    if (cronExpression !== schedule.cron_expression) {
      request.cron_expression = cronExpression;
    }
    if (maxConcurrent !== (schedule.max_concurrent?.toString() || "")) {
      request.max_concurrent = maxConcurrent ? parseInt(maxConcurrent, 10) : undefined;
    }
    if (catchUpMissed !== schedule.catch_up_missed) {
      request.catch_up_missed = catchUpMissed;
    }
    if (maxCatchUp !== (schedule.max_catch_up?.toString() || "")) {
      request.max_catch_up = maxCatchUp ? parseInt(maxCatchUp, 10) : undefined;
    }

    try {
      await updateMutation.mutateAsync({ scheduleId: schedule.id, request });
      onClose();
    } catch (err) {
      console.error("Failed to update schedule:", err);
    }
  };

  return (
    <DialogContent className="sm:max-w-[500px]">
      <DialogHeader>
        <DialogTitle>Edit Schedule</DialogTitle>
        <DialogDescription>Update the schedule configuration.</DialogDescription>
      </DialogHeader>
      <div className="grid gap-4 py-4">
        <div className="grid gap-2">
          <Label htmlFor="description">Description</Label>
          <Input
            id="description"
            placeholder="Description of this schedule"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
          />
        </div>
        <div className="grid gap-2">
          <Label htmlFor="cron">Cron Expression (7-field)</Label>
          <Input
            id="cron"
            placeholder="0 * * * * * *"
            value={cronExpression}
            onChange={(e) => setCronExpression(e.target.value)}
          />
          <p className="text-xs text-muted-foreground">
            Format: sec min hour day month day-of-week year
          </p>
        </div>
        <div className="grid gap-2">
          <Label htmlFor="maxConcurrent">Max Concurrent (optional)</Label>
          <Input
            id="maxConcurrent"
            type="number"
            placeholder="Leave empty for unlimited"
            value={maxConcurrent}
            onChange={(e) => setMaxConcurrent(e.target.value)}
          />
        </div>
        <div className="flex items-center gap-2">
          <Switch id="catchUpMissed" checked={catchUpMissed} onCheckedChange={setCatchUpMissed} />
          <Label htmlFor="catchUpMissed">Catch up missed triggers</Label>
        </div>
        {catchUpMissed && (
          <div className="grid gap-2">
            <Label htmlFor="maxCatchUp">Max Catch-Up Executions</Label>
            <Input
              id="maxCatchUp"
              type="number"
              placeholder="10"
              value={maxCatchUp}
              onChange={(e) => setMaxCatchUp(e.target.value)}
            />
          </div>
        )}
      </div>
      <DialogFooter>
        <Button variant="outline" onClick={onClose}>
          Cancel
        </Button>
        <Button onClick={handleSubmit} disabled={updateMutation.isPending}>
          {updateMutation.isPending ? "Saving..." : "Save Changes"}
        </Button>
      </DialogFooter>
    </DialogContent>
  );
}

export default function ScheduleDetailPage() {
  const params = useParams();
  const router = useRouter();
  const scheduleId = params.scheduleId as string;

  const [statusFilter, setStatusFilter] = useState<string>("all");
  const [editDialogOpen, setEditDialogOpen] = useState(false);

  const {
    data: schedule,
    isLoading: scheduleLoading,
    error: scheduleError,
    refetch,
  } = useSchedule(scheduleId);
  const executionsStatus = statusFilter !== "all" ? statusFilter : undefined;
  const {
    data: executionsData,
    isLoading: executionsLoading,
    refetch: refetchExecutions,
  } = useScheduleExecutions(scheduleId, { status: executionsStatus });
  const { data: stats, isLoading: statsLoading } = useScheduleStats(scheduleId);

  const pauseMutation = usePauseSchedule();
  const resumeMutation = useResumeSchedule();
  const triggerMutation = useTriggerSchedule();
  const deleteMutation = useDeleteSchedule();

  if (scheduleLoading) {
    return (
      <div className="container mx-auto p-6">
        <div className="flex items-center justify-between mb-6">
          <h1 className="text-2xl font-bold">Schedule Details</h1>
        </div>
        <div className="space-y-6">
          <Skeleton className="h-10 w-full" />
          <Skeleton className="h-48" />
          <Skeleton className="h-96" />
        </div>
      </div>
    );
  }

  if (scheduleError || !schedule) {
    return (
      <div className="container mx-auto p-6">
        <div className="flex items-center justify-between mb-6">
          <h1 className="text-2xl font-bold">Schedule Details</h1>
        </div>
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-12">
            <AlertTriangle className="h-12 w-12 text-muted-foreground mb-4" />
            <h3 className="text-lg font-medium mb-2">Schedule Not Found</h3>
            <p className="text-sm text-muted-foreground text-center max-w-md mb-4">
              The schedule could not be found or the API is not available.
            </p>
            <div className="flex gap-2">
              <Button onClick={() => refetch()} variant="outline">
                <RefreshCw className="h-4 w-4 mr-2" />
                Retry
              </Button>
              <Link href="/durable/schedules">
                <Button variant="outline">
                  <ArrowLeft className="h-4 w-4 mr-2" />
                  Back to Schedules
                </Button>
              </Link>
            </div>
          </CardContent>
        </Card>
      </div>
    );
  }

  const executions = executionsData?.data || [];

  const handlePause = () => {
    pauseMutation.mutate(scheduleId);
  };

  const handleResume = () => {
    resumeMutation.mutate(scheduleId);
  };

  const handleTrigger = () => {
    if (confirm("Manually trigger this schedule now?")) {
      triggerMutation.mutate(scheduleId);
    }
  };

  const handleDelete = () => {
    if (confirm("Are you sure you want to delete this schedule? This cannot be undone.")) {
      deleteMutation.mutate(scheduleId, {
        onSuccess: () => {
          router.push("/durable/schedules");
        },
      });
    }
  };

  return (
    <div className="container mx-auto p-6">
      <div className="flex items-center justify-between mb-6">
        <h1 className="text-2xl font-bold">{schedule.name}</h1>
      </div>
      <div className="space-y-6">
        {/* Back Link */}
        <Link
          href="/durable/schedules"
          className="inline-flex items-center gap-2 text-sm text-muted-foreground hover:text-foreground"
        >
          <ArrowLeft className="h-4 w-4" />
          Back to Schedules
        </Link>

        {/* Schedule Overview */}
        <Card>
          <CardHeader>
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-3">
                <Clock
                  className={`h-6 w-6 ${schedule.enabled ? "text-green-500" : "text-gray-400"}`}
                />
                <div>
                  <CardTitle className="flex items-center gap-2">
                    {schedule.name}
                    <CopyButton value={schedule.id} />
                  </CardTitle>
                  {schedule.description && (
                    <CardDescription>{schedule.description}</CardDescription>
                  )}
                </div>
              </div>
              <div className="flex items-center gap-2">
                <Badge variant={schedule.enabled ? "default" : "outline"}>
                  {schedule.enabled ? "Active" : "Paused"}
                </Badge>
                <Button variant="outline" size="sm" onClick={handleTrigger}>
                  <Zap className="h-4 w-4 mr-1" />
                  Trigger
                </Button>
                {schedule.enabled ? (
                  <Button variant="outline" size="sm" onClick={handlePause}>
                    <Pause className="h-4 w-4 mr-1" />
                    Pause
                  </Button>
                ) : (
                  <Button variant="outline" size="sm" onClick={handleResume}>
                    <Play className="h-4 w-4 mr-1" />
                    Resume
                  </Button>
                )}
                <Dialog open={editDialogOpen} onOpenChange={setEditDialogOpen}>
                  <DialogTrigger asChild>
                    <Button variant="outline" size="sm">
                      <Settings className="h-4 w-4 mr-1" />
                      Edit
                    </Button>
                  </DialogTrigger>
                  <EditScheduleDialog
                    schedule={schedule}
                    onClose={() => setEditDialogOpen(false)}
                  />
                </Dialog>
                <Button
                  variant="outline"
                  size="sm"
                  className="text-destructive hover:text-destructive"
                  onClick={handleDelete}
                >
                  <Trash2 className="h-4 w-4 mr-1" />
                  Delete
                </Button>
              </div>
            </div>
          </CardHeader>
          <CardContent>
            <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
              <div>
                <p className="text-sm text-muted-foreground">Cron Expression</p>
                <code className="text-sm bg-muted px-2 py-0.5 rounded">
                  {schedule.cron_expression}
                </code>
              </div>
              <div>
                <p className="text-sm text-muted-foreground">Timezone</p>
                <p className="text-sm font-medium">{schedule.timezone}</p>
              </div>
              <div>
                <p className="text-sm text-muted-foreground">Target</p>
                <div className="flex items-center gap-2">
                  <Badge variant="outline">{schedule.target.type}</Badge>
                  <span className="text-sm font-medium">{schedule.target.name}</span>
                </div>
              </div>
              <div>
                <p className="text-sm text-muted-foreground">Max Concurrent</p>
                <p className="text-sm font-medium">{schedule.max_concurrent ?? "Unlimited"}</p>
              </div>
              <div>
                <p className="text-sm text-muted-foreground">Last Triggered</p>
                <p className="text-sm font-medium">
                  {schedule.last_triggered_at
                    ? formatDistanceToNow(new Date(schedule.last_triggered_at), { addSuffix: true })
                    : "Never"}
                </p>
              </div>
              <div>
                <p className="text-sm text-muted-foreground">Next Trigger</p>
                <p className="text-sm font-medium">
                  {schedule.next_trigger_at && schedule.enabled
                    ? formatDistanceToNow(new Date(schedule.next_trigger_at), { addSuffix: true })
                    : "-"}
                </p>
              </div>
              <div>
                <p className="text-sm text-muted-foreground">Created</p>
                <p className="text-sm font-medium">
                  {new Date(schedule.created_at).toLocaleDateString()}
                </p>
              </div>
              <div>
                <p className="text-sm text-muted-foreground">Catch Up Missed</p>
                <p className="text-sm font-medium">
                  {schedule.catch_up_missed ? `Yes (max ${schedule.max_catch_up ?? 10})` : "No"}
                </p>
              </div>
            </div>

            {/* Target Input */}
            {Object.keys(schedule.target.input).length > 0 && (
              <div className="mt-4">
                <p className="text-sm text-muted-foreground mb-2">Target Input</p>
                <pre className="text-xs bg-muted p-3 rounded overflow-auto max-h-32">
                  {JSON.stringify(schedule.target.input, null, 2)}
                </pre>
              </div>
            )}
          </CardContent>
        </Card>

        {/* Statistics */}
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2 text-lg">
              <BarChart3 className="h-5 w-5" />
              Statistics
            </CardTitle>
          </CardHeader>
          <CardContent>
            {statsLoading ? (
              <div className="grid grid-cols-2 md:grid-cols-5 gap-4">
                {[...Array(5)].map((_, i) => (
                  <Skeleton key={i} className="h-16" />
                ))}
              </div>
            ) : stats ? (
              <div className="grid grid-cols-2 md:grid-cols-5 gap-4">
                <div className="text-center p-4 bg-muted">
                  <p className="text-2xl font-bold">{stats.total_executions}</p>
                  <p className="text-sm text-muted-foreground">Total Executions</p>
                </div>
                <div className="text-center p-4 bg-muted">
                  <p className="text-2xl font-bold text-green-500">{stats.successful_executions}</p>
                  <p className="text-sm text-muted-foreground">Successful</p>
                </div>
                <div className="text-center p-4 bg-muted">
                  <p className="text-2xl font-bold text-red-500">{stats.failed_executions}</p>
                  <p className="text-sm text-muted-foreground">Failed</p>
                </div>
                <div className="text-center p-4 bg-muted">
                  <p className="text-2xl font-bold text-yellow-500">{stats.skipped_executions}</p>
                  <p className="text-sm text-muted-foreground">Skipped</p>
                </div>
                <div className="text-center p-4 bg-muted">
                  <p className="text-2xl font-bold">
                    {stats.avg_duration_ms ? `${stats.avg_duration_ms}ms` : "-"}
                  </p>
                  <p className="text-sm text-muted-foreground">Avg Duration</p>
                </div>
              </div>
            ) : (
              <p className="text-sm text-muted-foreground">No statistics available</p>
            )}
          </CardContent>
        </Card>

        {/* Execution History */}
        <Card>
          <CardHeader>
            <div className="flex items-center justify-between">
              <CardTitle className="flex items-center gap-2 text-lg">
                <History className="h-5 w-5" />
                Execution History
              </CardTitle>
              <div className="flex items-center gap-2">
                <Select value={statusFilter} onValueChange={setStatusFilter}>
                  <SelectTrigger className="w-32">
                    <SelectValue placeholder="Status" />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">All</SelectItem>
                    <SelectItem value="pending">Pending</SelectItem>
                    <SelectItem value="running">Running</SelectItem>
                    <SelectItem value="completed">Completed</SelectItem>
                    <SelectItem value="failed">Failed</SelectItem>
                    <SelectItem value="skipped">Skipped</SelectItem>
                  </SelectContent>
                </Select>
                <Button variant="outline" size="sm" onClick={() => refetchExecutions()}>
                  <RefreshCw className="h-4 w-4" />
                </Button>
              </div>
            </div>
          </CardHeader>
          <CardContent>
            {executionsLoading ? (
              <Skeleton className="h-48" />
            ) : executions.length > 0 ? (
              <div className="border">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Status</TableHead>
                      <TableHead>Scheduled</TableHead>
                      <TableHead>Started</TableHead>
                      <TableHead>Completed</TableHead>
                      <TableHead>Duration</TableHead>
                      <TableHead>Workflow</TableHead>
                      <TableHead>Error</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {executions.map((execution) => (
                      <ExecutionRow key={execution.id} execution={execution} />
                    ))}
                  </TableBody>
                </Table>
              </div>
            ) : (
              <div className="flex flex-col items-center justify-center py-12 text-muted-foreground">
                <History className="h-12 w-12 mb-4" />
                <h3 className="text-lg font-medium mb-2">No Executions Yet</h3>
                <p className="text-sm text-center max-w-md">
                  {statusFilter !== "all"
                    ? "No executions match your filter criteria."
                    : "This schedule hasn't been triggered yet. Click 'Trigger' to run it manually."}
                </p>
              </div>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
