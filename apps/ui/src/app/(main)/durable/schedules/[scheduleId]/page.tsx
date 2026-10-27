"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import {
  useDurablePageAccess,
  useDurablePageRows,
} from "@/components/chat-mode/use-durable-page-rows";

import { useChatPageFilters } from "@/components/chat-mode/page-view-state";
import { usePageFormDraft } from "@/components/chat-mode/use-page-form-draft";
import { scheduleEditDraftSchema } from "@/components/chat-mode/page-form-schemas";

import { usePageFormPrefill } from "@/components/chat-mode/use-page-form-prefill";
import { useState } from "react";
import { useParams, useRouter } from "next/navigation";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button, LinkButton } from "@/components/ui/button";
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
  usePageTitle,
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
import { EntityIdentity } from "@/components/ui/entity-identity";
import { formatDistanceToNow } from "@/lib/formatting";
import { getScheduleExecutionBadgeVariant } from "@/lib/status-utils";

function getStatusIcon(status: ScheduleExecutionStatus) {
  switch (status) {
    case "completed":
      return <CheckCircle className="h-4 w-4 text-success" />;
    case "running":
      return <Activity className="h-4 w-4 text-info animate-pulse" />;
    case "failed":
      return <XCircle className="h-4 w-4 text-destructive" />;
    case "skipped":
      return <SkipForward className="h-4 w-4 text-warning" />;
    case "pending":
    default:
      return <Clock className="h-4 w-4 text-muted-foreground" />;
  }
}

function ExecutionRow({ execution }: { execution: ScheduleExecution }) {
  const ui = useUiTranslations();
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
          <span className="text-sm">
            {execution.duration_ms}
            {ui("ms")}
          </span>
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
            {ui("View Workflow")}
          </Link>
        ) : (
          <span className="text-sm text-muted-foreground">-</span>
        )}
      </TableCell>
      <TableCell>
        {execution.error ? (
          <TooltipProvider>
            <Tooltip>
              <TooltipTrigger className="text-sm text-destructive max-w-[200px] truncate block">
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
  open,
  schedule,
  onClose,
}: {
  open: boolean;
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
  const ui = useUiTranslations();
  const [description, setDescription] = useState(schedule.description || "");
  const [cronExpression, setCronExpression] = useState(schedule.cron_expression);
  const [maxConcurrent, setMaxConcurrent] = useState(schedule.max_concurrent?.toString() || "");
  const [catchUpMissed, setCatchUpMissed] = useState(schedule.catch_up_missed);
  const [maxCatchUp, setMaxCatchUp] = useState(schedule.max_catch_up?.toString() || "");

  const updateMutation = useUpdateSchedule();
  const values = {
    description,
    cron_expression: cronExpression,
    max_concurrent: maxConcurrent,
    catch_up_missed: catchUpMissed,
    max_catch_up: maxCatchUp,
  };
  const saved = {
    description: schedule.description || "",
    cron_expression: schedule.cron_expression,
    max_concurrent: schedule.max_concurrent?.toString() || "",
    catch_up_missed: schedule.catch_up_missed,
    max_catch_up: schedule.max_catch_up?.toString() || "",
  };
  const applyValues = (next: typeof values) => {
    setDescription(next.description);
    setCronExpression(next.cron_expression);
    setMaxConcurrent(next.max_concurrent);
    setCatchUpMissed(next.catch_up_missed);
    setMaxCatchUp(next.max_catch_up);
  };
  usePageFormPrefill({ identity: schedule.id, open, values, saved, apply: applyValues });
  const { manage: canManageDraft } = useDurablePageAccess();
  const chat = usePageFormDraft({
    schema: scheduleEditDraftSchema,
    entityType: "schedule",
    entityId: schedule.id,
    label: ui("Edit schedule"),
    priority: 30,
    enabled: open,
    values,
    baseline: JSON.stringify(saved),
    dirty: JSON.stringify(values) !== JSON.stringify(saved),
    writable: canManageDraft && !updateMutation.isPending,
    apply: applyValues,
  });

  const handleSubmit = async () => {
    if (chat.blocked) return;
    const request: UpdateScheduleRequest = {};

    if (description !== (schedule.description || "")) {
      request.description = description || (chat.adopted ? null : undefined);
    }
    if (cronExpression !== schedule.cron_expression) {
      request.cron_expression = cronExpression;
    }
    if (maxConcurrent !== (schedule.max_concurrent?.toString() || "")) {
      request.max_concurrent = maxConcurrent
        ? parseInt(maxConcurrent, 10)
        : chat.adopted
          ? null
          : undefined;
    }
    if (catchUpMissed !== schedule.catch_up_missed) {
      request.catch_up_missed = catchUpMissed;
    }
    if (maxCatchUp !== (schedule.max_catch_up?.toString() || "")) {
      request.max_catch_up = maxCatchUp
        ? parseInt(maxCatchUp, 10)
        : chat.adopted
          ? null
          : undefined;
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
        <DialogTitle>{ui("Edit Schedule")}</DialogTitle>
        <DialogDescription>{ui("Update the schedule configuration.")}</DialogDescription>
      </DialogHeader>
      {chat.notice}
      {chat.adopted && chat.blocked && (
        <Button
          type="button"
          variant="outline"
          onClick={() => {
            applyValues(saved);
            chat.clear();
          }}
        >
          {ui("Discard changes")}
        </Button>
      )}
      <div className="grid gap-4 py-4">
        <div className="grid gap-2">
          <Label htmlFor="description">{ui("Description")}</Label>
          <Input
            id="description"
            placeholder={ui("Description of this schedule")}
            value={description}
            onChange={(e) => setDescription(e.target.value)}
          />
        </div>
        <div className="grid gap-2">
          <Label htmlFor="cron">{ui("Cron Expression (7-field)")}</Label>
          <Input
            id="cron"
            placeholder="0 * * * * * *"
            value={cronExpression}
            onChange={(e) => setCronExpression(e.target.value)}
          />
          <p className="text-xs text-muted-foreground">
            {ui("Format: sec min hour day month day-of-week year")}
          </p>
        </div>
        <div className="grid gap-2">
          <Label htmlFor="maxConcurrent">{ui("Max Concurrent (optional)")}</Label>
          <Input
            id="maxConcurrent"
            type="number"
            placeholder={ui("Leave empty for unlimited")}
            value={maxConcurrent}
            onChange={(e) => setMaxConcurrent(e.target.value)}
          />
        </div>
        <div className="flex items-center gap-2">
          <Switch id="catchUpMissed" checked={catchUpMissed} onCheckedChange={setCatchUpMissed} />
          <Label htmlFor="catchUpMissed">{ui("Catch up missed triggers")}</Label>
        </div>
        {catchUpMissed && (
          <div className="grid gap-2">
            <Label htmlFor="maxCatchUp">{ui("Max Catch-Up Executions")}</Label>
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
          {ui("Cancel")}
        </Button>
        <Button onClick={handleSubmit} disabled={chat.blocked || updateMutation.isPending}>
          {updateMutation.isPending ? ui("Saving...") : ui("Save Changes")}
        </Button>
      </DialogFooter>
    </DialogContent>
  );
}

export default function ScheduleDetailPage() {
  const ui = useUiTranslations();
  const params = useParams();
  const router = useRouter();
  const scheduleId = params.scheduleId as string;

  const [statusFilter, setStatusFilter] = useState<string>("all");
  useChatPageFilters({ status: statusFilter });
  const [editDialogOpen, setEditDialogOpen] = useState(false);

  const {
    data: schedule,
    isLoading: scheduleLoading,
    error: scheduleError,
    refetch,
  } = useSchedule(scheduleId);
  usePageTitle(schedule?.name ?? null, ui("Schedule"), ui("Durable"));
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

  useDurablePageRows(
    "Schedule",
    schedule
      ? [
          {
            id: schedule.id,
            name: schedule.name,
            cron_expression: schedule.cron_expression,
            enabled: schedule.enabled,
            target_type: schedule.target.type,
            target_name: schedule.target.name,
          },
        ]
      : [],
    { loading: scheduleLoading, error: !!scheduleError },
  );
  useDurablePageRows(
    "Schedule executions",
    (executionsData?.data ?? []).map((e) => ({
      id: e.id,
      status: e.status,
      scheduled_at: e.scheduled_at,
      started_at: e.started_at,
      completed_at: e.completed_at,
    })),
    { loading: executionsLoading },
  );

  if (scheduleLoading) {
    return (
      <div className="operational-page container mx-auto p-6">
        <div className="operational-toolbar flex flex-wrap items-center justify-between gap-3 mb-6">
          <h1 className="text-2xl font-bold">{ui("Schedule Details")}</h1>
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
      <div className="operational-page container mx-auto p-6">
        <div className="operational-toolbar flex flex-wrap items-center justify-between gap-3 mb-6">
          <h1 className="text-2xl font-bold">{ui("Schedule Details")}</h1>
        </div>
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-12">
            <AlertTriangle className="h-12 w-12 text-muted-foreground mb-4" />
            <h3 className="text-lg font-medium mb-2">{ui("Schedule Not Found")}</h3>
            <p className="text-sm text-muted-foreground text-center max-w-md mb-4">
              {ui("The schedule could not be found or the API is not available.")}
            </p>
            <div className="flex gap-2">
              <Button onClick={() => refetch()} variant="outline">
                <RefreshCw className="h-4 w-4 mr-2" />
                {ui("Retry")}
              </Button>
              <LinkButton variant="outline" href="/durable/schedules">
                <ArrowLeft className="h-4 w-4 mr-2" />
                {ui("Back to Schedules")}
              </LinkButton>
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
    <div className="operational-page container mx-auto p-6">
      <div className="operational-toolbar flex flex-wrap items-center justify-between gap-3 mb-6">
        <h1 className="text-2xl font-bold">{schedule.name}</h1>
      </div>
      <div className="space-y-6">
        {/* Back Link */}
        <Link
          href="/durable/schedules"
          className="inline-flex items-center gap-2 text-sm text-muted-foreground hover:text-foreground"
        >
          <ArrowLeft className="h-4 w-4" />
          {ui("Back to Schedules")}
        </Link>

        {/* Schedule Overview */}
        <Card>
          <CardHeader>
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-3">
                <Clock
                  className={`h-6 w-6 ${schedule.enabled ? "text-success" : "text-muted-foreground"}`}
                />
                <div>
                  <CardTitle>
                    <EntityIdentity value={schedule.id}>{schedule.name}</EntityIdentity>
                  </CardTitle>
                  {schedule.description && (
                    <CardDescription>{schedule.description}</CardDescription>
                  )}
                </div>
              </div>
              <div className="flex items-center gap-2">
                <Badge variant={schedule.enabled ? "default" : "outline"}>
                  {schedule.enabled ? ui("Active") : ui("Paused")}
                </Badge>
                <Button variant="outline" size="sm" onClick={handleTrigger}>
                  <Zap className="h-4 w-4 mr-1" />
                  {ui("Trigger")}
                </Button>
                {schedule.enabled ? (
                  <Button variant="outline" size="sm" onClick={handlePause}>
                    <Pause className="h-4 w-4 mr-1" />
                    {ui("Pause")}
                  </Button>
                ) : (
                  <Button variant="outline" size="sm" onClick={handleResume}>
                    <Play className="h-4 w-4 mr-1" />
                    {ui("Resume")}
                  </Button>
                )}
                <Dialog open={editDialogOpen} onOpenChange={setEditDialogOpen}>
                  <DialogTrigger asChild>
                    <Button variant="outline" size="sm">
                      <Settings className="h-4 w-4 mr-1" />
                      {ui("Edit")}
                    </Button>
                  </DialogTrigger>
                  <EditScheduleDialog
                    open={editDialogOpen}
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
                  {ui("Delete")}
                </Button>
              </div>
            </div>
          </CardHeader>
          <CardContent>
            <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
              <div>
                <p className="text-sm text-muted-foreground">{ui("Cron Expression")}</p>
                <code className="text-sm bg-muted px-2 py-0.5 rounded">
                  {schedule.cron_expression}
                </code>
              </div>
              <div>
                <p className="text-sm text-muted-foreground">{ui("Timezone")}</p>
                <p className="text-sm font-medium">{schedule.timezone}</p>
              </div>
              <div>
                <p className="text-sm text-muted-foreground">{ui("Target")}</p>
                <div className="flex items-center gap-2">
                  <Badge variant="outline">{schedule.target.type}</Badge>
                  <span className="text-sm font-medium">{schedule.target.name}</span>
                </div>
              </div>
              <div>
                <p className="text-sm text-muted-foreground">{ui("Max Concurrent")}</p>
                <p className="text-sm font-medium">{schedule.max_concurrent ?? ui("Unlimited")}</p>
              </div>
              <div>
                <p className="text-sm text-muted-foreground">{ui("Last Triggered")}</p>
                <p className="text-sm font-medium">
                  {schedule.last_triggered_at
                    ? formatDistanceToNow(new Date(schedule.last_triggered_at), { addSuffix: true })
                    : ui("Never")}
                </p>
              </div>
              <div>
                <p className="text-sm text-muted-foreground">{ui("Next Trigger")}</p>
                <p className="text-sm font-medium">
                  {schedule.next_trigger_at && schedule.enabled
                    ? formatDistanceToNow(new Date(schedule.next_trigger_at), { addSuffix: true })
                    : "-"}
                </p>
              </div>
              <div>
                <p className="text-sm text-muted-foreground">{ui("Created")}</p>
                <p className="text-sm font-medium">
                  {new Date(schedule.created_at).toLocaleDateString()}
                </p>
              </div>
              <div>
                <p className="text-sm text-muted-foreground">{ui("Catch Up Missed")}</p>
                <p className="text-sm font-medium">
                  {schedule.catch_up_missed
                    ? ui("Yes (max {value0})", { value0: schedule.max_catch_up ?? 10 })
                    : ui("No")}
                </p>
              </div>
            </div>

            {/* Target Input */}
            {Object.keys(schedule.target.input).length > 0 && (
              <div className="mt-4">
                <p className="text-sm text-muted-foreground mb-2">{ui("Target Input")}</p>
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
              {ui("Statistics")}
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
                  <p className="text-sm text-muted-foreground">{ui("Total Executions")}</p>
                </div>
                <div className="text-center p-4 bg-muted">
                  <p className="text-2xl font-bold text-success">{stats.successful_executions}</p>
                  <p className="text-sm text-muted-foreground">{ui("Successful")}</p>
                </div>
                <div className="text-center p-4 bg-muted">
                  <p className="text-2xl font-bold text-destructive">{stats.failed_executions}</p>
                  <p className="text-sm text-muted-foreground">{ui("Failed")}</p>
                </div>
                <div className="text-center p-4 bg-muted">
                  <p className="text-2xl font-bold text-warning">{stats.skipped_executions}</p>
                  <p className="text-sm text-muted-foreground">{ui("Skipped")}</p>
                </div>
                <div className="text-center p-4 bg-muted">
                  <p className="text-2xl font-bold">
                    {stats.avg_duration_ms
                      ? ui("{value0}ms", { value0: stats.avg_duration_ms })
                      : "-"}
                  </p>
                  <p className="text-sm text-muted-foreground">{ui("Avg Duration")}</p>
                </div>
              </div>
            ) : (
              <p className="text-sm text-muted-foreground">{ui("No statistics available")}</p>
            )}
          </CardContent>
        </Card>

        {/* Execution History */}
        <Card>
          <CardHeader>
            <div className="flex items-center justify-between">
              <CardTitle className="flex items-center gap-2 text-lg">
                <History className="h-5 w-5" />
                {ui("Execution History")}
              </CardTitle>
              <div className="flex items-center gap-2">
                <Select value={statusFilter} onValueChange={setStatusFilter}>
                  <SelectTrigger className="w-32">
                    <SelectValue placeholder={ui("Status")} />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">{ui("All")}</SelectItem>
                    <SelectItem value="pending">{ui("Pending")}</SelectItem>
                    <SelectItem value="running">{ui("Running")}</SelectItem>
                    <SelectItem value="completed">{ui("Completed")}</SelectItem>
                    <SelectItem value="failed">{ui("Failed")}</SelectItem>
                    <SelectItem value="skipped">{ui("Skipped")}</SelectItem>
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
                      <TableHead>{ui("Status")}</TableHead>
                      <TableHead>{ui("Scheduled")}</TableHead>
                      <TableHead>{ui("Started")}</TableHead>
                      <TableHead>{ui("Completed")}</TableHead>
                      <TableHead>{ui("Duration")}</TableHead>
                      <TableHead>{ui("Workflow")}</TableHead>
                      <TableHead>{ui("Error")}</TableHead>
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
                <h3 className="text-lg font-medium mb-2">{ui("No Executions Yet")}</h3>
                <p className="text-sm text-center max-w-md">
                  {statusFilter !== "all"
                    ? ui("No executions match your filter criteria.")
                    : ui(
                        "This schedule hasn't been triggered yet. Click 'Trigger' to run it manually.",
                      )}
                </p>
              </div>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
