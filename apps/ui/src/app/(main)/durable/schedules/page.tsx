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
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Switch } from "@/components/ui/switch";
import {
  useSchedules,
  useCreateSchedule,
  useDeleteSchedule,
  usePauseSchedule,
  useResumeSchedule,
  useTriggerSchedule,
} from "@/hooks";
import type { DurableSchedule, CreateScheduleRequest } from "@/lib/api/types";
import {
  Clock,
  AlertTriangle,
  Plus,
  Search,
  RefreshCw,
  Play,
  Pause,
  Trash2,
  Zap,
  Calendar,
  Settings,
} from "lucide-react";
import Link from "next/link";
import { CopyButton } from "@/components/ui/copy-button";

function formatDistanceToNow(date: Date, options?: { addSuffix?: boolean }): string {
  const now = new Date();
  const diff = now.getTime() - date.getTime();
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

  if (options?.addSuffix) {
    return diff > 0 ? `${result} ago` : `in ${result}`;
  }
  return result;
}

function ScheduleRow({
  schedule,
  onPause,
  onResume,
  onTrigger,
  onDelete,
}: {
  schedule: DurableSchedule;
  onPause: (id: string) => void;
  onResume: (id: string) => void;
  onTrigger: (id: string) => void;
  onDelete: (id: string) => void;
}) {
  return (
    <TableRow>
      <TableCell>
        <div className="flex items-center gap-2 min-w-0">
          <Clock
            className={`h-4 w-4 shrink-0 ${schedule.enabled ? "text-green-500" : "text-gray-400"}`}
          />
          <div className="min-w-0">
            <Link
              href={`/durable/schedules/${schedule.id}`}
              className="font-medium hover:underline truncate block"
            >
              {schedule.name}
            </Link>
            {schedule.description && (
              <p className="text-xs text-muted-foreground truncate">{schedule.description}</p>
            )}
          </div>
          <CopyButton value={schedule.id} />
        </div>
      </TableCell>
      <TableCell>
        <Badge variant={schedule.enabled ? "default" : "outline"}>
          {schedule.enabled ? "Active" : "Paused"}
        </Badge>
      </TableCell>
      <TableCell>
        <code className="text-sm bg-muted px-2 py-0.5 rounded">{schedule.cron_expression}</code>
      </TableCell>
      <TableCell>
        <div className="flex items-center gap-2 min-w-0">
          <Badge variant="outline" className="shrink-0">
            {schedule.target.type}
          </Badge>
          <span className="text-sm truncate">{schedule.target.name}</span>
        </div>
      </TableCell>
      <TableCell>
        {schedule.last_triggered_at ? (
          <TooltipProvider>
            <Tooltip>
              <TooltipTrigger className="text-sm text-muted-foreground">
                {formatDistanceToNow(new Date(schedule.last_triggered_at), { addSuffix: true })}
              </TooltipTrigger>
              <TooltipContent>
                {new Date(schedule.last_triggered_at).toLocaleString()}
              </TooltipContent>
            </Tooltip>
          </TooltipProvider>
        ) : (
          <span className="text-sm text-muted-foreground">Never</span>
        )}
      </TableCell>
      <TableCell>
        {schedule.next_trigger_at && schedule.enabled ? (
          <TooltipProvider>
            <Tooltip>
              <TooltipTrigger className="text-sm text-muted-foreground">
                {formatDistanceToNow(new Date(schedule.next_trigger_at), { addSuffix: true })}
              </TooltipTrigger>
              <TooltipContent>{new Date(schedule.next_trigger_at).toLocaleString()}</TooltipContent>
            </Tooltip>
          </TooltipProvider>
        ) : (
          <span className="text-sm text-muted-foreground">-</span>
        )}
      </TableCell>
      <TableCell>
        <div className="flex items-center gap-1">
          <TooltipProvider>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button variant="ghost" size="sm" onClick={() => onTrigger(schedule.id)}>
                  <Zap className="h-3 w-3" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>Trigger Now</TooltipContent>
            </Tooltip>
          </TooltipProvider>
          {schedule.enabled ? (
            <TooltipProvider>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button variant="ghost" size="sm" onClick={() => onPause(schedule.id)}>
                    <Pause className="h-3 w-3" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>Pause</TooltipContent>
              </Tooltip>
            </TooltipProvider>
          ) : (
            <TooltipProvider>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button variant="ghost" size="sm" onClick={() => onResume(schedule.id)}>
                    <Play className="h-3 w-3" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>Resume</TooltipContent>
              </Tooltip>
            </TooltipProvider>
          )}
          <Link href={`/durable/schedules/${schedule.id}`}>
            <Button variant="ghost" size="sm">
              <Settings className="h-3 w-3" />
            </Button>
          </Link>
          <TooltipProvider>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="sm"
                  className="text-destructive hover:text-destructive"
                  onClick={() => onDelete(schedule.id)}
                >
                  <Trash2 className="h-3 w-3" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>Delete</TooltipContent>
            </Tooltip>
          </TooltipProvider>
        </div>
      </TableCell>
    </TableRow>
  );
}

function CreateScheduleDialog({ onClose }: { onClose: () => void }) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [cronExpression, setCronExpression] = useState("0 * * * * * *");
  const [targetType, setTargetType] = useState<"workflow" | "activity">("workflow");
  const [targetName, setTargetName] = useState("");
  const [targetInput, setTargetInput] = useState("{}");
  const [enabled, setEnabled] = useState(true);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  const createMutation = useCreateSchedule();

  const handleSubmit = async () => {
    if (!name || !cronExpression || !targetName) return;

    setErrorMessage(null);

    let input: Record<string, unknown> = {};
    try {
      input = JSON.parse(targetInput);
    } catch {
      setErrorMessage("Invalid JSON in target input");
      return;
    }

    const request: CreateScheduleRequest = {
      name,
      description: description || undefined,
      cron_expression: cronExpression,
      target: {
        type: targetType,
        name: targetName,
        input,
      },
      enabled,
    };

    try {
      await createMutation.mutateAsync(request);
      onClose();
    } catch (err) {
      const message =
        err instanceof Error ? err.message : "Failed to create schedule";
      setErrorMessage(message);
    }
  };

  return (
    <DialogContent className="sm:max-w-[500px]">
      <DialogHeader>
        <DialogTitle>Create Schedule</DialogTitle>
        <DialogDescription>
          Create a new scheduled task to trigger workflows or activities automatically.
        </DialogDescription>
      </DialogHeader>
      <div className="grid gap-4 py-4">
        <div className="grid gap-2">
          <Label htmlFor="name">Name</Label>
          <Input
            id="name"
            placeholder="my-schedule"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </div>
        <div className="grid gap-2">
          <Label htmlFor="description">Description (optional)</Label>
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
            Format: sec min hour day month day-of-week year (e.g., &quot;0 0 * * * * *&quot; for
            every hour)
          </p>
        </div>
        <div className="grid gap-2">
          <Label>Target Type</Label>
          <Select
            value={targetType}
            onValueChange={(v) => setTargetType(v as "workflow" | "activity")}
          >
            <SelectTrigger>
              <SelectValue>{targetType === "workflow" ? "Workflow" : "Activity"}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="workflow">Workflow</SelectItem>
              <SelectItem value="activity">Activity</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div className="grid gap-2">
          <Label htmlFor="targetName">Target Name</Label>
          <Input
            id="targetName"
            placeholder={targetType === "workflow" ? "workflow-type-name" : "activity-type-name"}
            value={targetName}
            onChange={(e) => setTargetName(e.target.value)}
          />
        </div>
        <div className="grid gap-2">
          <Label htmlFor="targetInput">Target Input (JSON)</Label>
          <Textarea
            id="targetInput"
            placeholder="{}"
            value={targetInput}
            onChange={(e) => setTargetInput(e.target.value)}
            className="font-mono text-sm"
          />
        </div>
        <div className="flex items-center gap-2">
          <Switch id="enabled" checked={enabled} onCheckedChange={setEnabled} />
          <Label htmlFor="enabled">Enable schedule immediately</Label>
        </div>
      </div>
      {errorMessage && (
        <div className="rounded-md bg-destructive/10 border border-destructive/20 p-3">
          <p className="text-sm text-destructive">{errorMessage}</p>
        </div>
      )}
      <DialogFooter>
        <Button variant="outline" onClick={onClose} disabled={createMutation.isPending}>
          Cancel
        </Button>
        <Button
          onClick={handleSubmit}
          disabled={!name || !cronExpression || !targetName || createMutation.isPending}
        >
          {createMutation.isPending ? "Creating..." : "Create Schedule"}
        </Button>
      </DialogFooter>
    </DialogContent>
  );
}

export default function SchedulesPage() {
  const [statusFilter, setStatusFilter] = useState<string>("all");
  const [searchQuery, setSearchQuery] = useState("");
  const [createDialogOpen, setCreateDialogOpen] = useState(false);

  const enabledFilter =
    statusFilter === "active" ? true : statusFilter === "paused" ? false : undefined;
  const {
    data: schedulesData,
    isLoading,
    error,
    refetch,
  } = useSchedules({ enabled: enabledFilter });

  const pauseMutation = usePauseSchedule();
  const resumeMutation = useResumeSchedule();
  const triggerMutation = useTriggerSchedule();
  const deleteMutation = useDeleteSchedule();

  if (isLoading) {
    return (
      <div className="container mx-auto p-6">
        <div className="flex items-center justify-between mb-6">
          <h1 className="text-2xl font-bold">Schedules</h1>
        </div>
        <div className="space-y-6">
          <Skeleton className="h-10 w-full" />
          <Skeleton className="h-96" />
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="container mx-auto p-6">
        <div className="flex items-center justify-between mb-6">
          <h1 className="text-2xl font-bold">Schedules</h1>
        </div>
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-12">
            <AlertTriangle className="h-12 w-12 text-muted-foreground mb-4" />
            <h3 className="text-lg font-medium mb-2">Unable to Load Schedules</h3>
            <p className="text-sm text-muted-foreground text-center max-w-md mb-4">
              The durable schedules API is not available.
            </p>
            <Button onClick={() => refetch()} variant="outline">
              <RefreshCw className="h-4 w-4 mr-2" />
              Retry
            </Button>
          </CardContent>
        </Card>
      </div>
    );
  }

  const schedules = schedulesData?.data || [];
  const filteredSchedules = searchQuery
    ? schedules.filter(
        (s) =>
          s.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
          s.target.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
          s.id.toLowerCase().includes(searchQuery.toLowerCase()),
      )
    : schedules;

  const handlePause = (scheduleId: string) => {
    pauseMutation.mutate(scheduleId);
  };

  const handleResume = (scheduleId: string) => {
    resumeMutation.mutate(scheduleId);
  };

  const handleTrigger = (scheduleId: string) => {
    if (confirm("Manually trigger this schedule now?")) {
      triggerMutation.mutate(scheduleId);
    }
  };

  const handleDelete = (scheduleId: string) => {
    if (confirm("Are you sure you want to delete this schedule? This cannot be undone.")) {
      deleteMutation.mutate(scheduleId);
    }
  };

  return (
    <div className="container mx-auto p-6 overflow-hidden">
      <div className="flex items-center justify-between mb-6">
        <h1 className="text-2xl font-bold">Schedules</h1>
      </div>
      <div className="space-y-6">
        {/* Filters and Actions */}
        <div className="flex items-center justify-between gap-4">
          <div className="flex items-center gap-4 flex-1">
            <div className="relative flex-1 max-w-sm">
              <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
              <Input
                placeholder="Search schedules..."
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
                <SelectItem value="all">All</SelectItem>
                <SelectItem value="active">Active</SelectItem>
                <SelectItem value="paused">Paused</SelectItem>
              </SelectContent>
            </Select>
            <Button variant="outline" size="sm" onClick={() => refetch()}>
              <RefreshCw className="h-4 w-4 mr-2" />
              Refresh
            </Button>
          </div>
          <Dialog open={createDialogOpen} onOpenChange={setCreateDialogOpen}>
            <DialogTrigger asChild>
              <Button>
                <Plus className="h-4 w-4 mr-2" />
                New Schedule
              </Button>
            </DialogTrigger>
            <CreateScheduleDialog onClose={() => setCreateDialogOpen(false)} />
          </Dialog>
        </div>

        {/* Schedules Table */}
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Calendar className="h-5 w-5" />
              Scheduled Tasks
            </CardTitle>
            <CardDescription>
              Cron-based schedules that automatically trigger workflows or activities
            </CardDescription>
          </CardHeader>
          <CardContent>
            {filteredSchedules.length > 0 ? (
              <div className="rounded-md border overflow-x-auto">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead className="min-w-[200px]">Schedule</TableHead>
                      <TableHead className="w-[80px]">Status</TableHead>
                      <TableHead className="w-[140px]">Cron</TableHead>
                      <TableHead className="min-w-[150px]">Target</TableHead>
                      <TableHead className="w-[140px] whitespace-nowrap">Last Triggered</TableHead>
                      <TableHead className="w-[140px] whitespace-nowrap">Next Trigger</TableHead>
                      <TableHead className="w-[130px]">Actions</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {filteredSchedules.map((schedule) => (
                      <ScheduleRow
                        key={schedule.id}
                        schedule={schedule}
                        onPause={handlePause}
                        onResume={handleResume}
                        onTrigger={handleTrigger}
                        onDelete={handleDelete}
                      />
                    ))}
                  </TableBody>
                </Table>
              </div>
            ) : (
              <div className="flex flex-col items-center justify-center py-12 text-muted-foreground">
                <Calendar className="h-12 w-12 mb-4" />
                <h3 className="text-lg font-medium mb-2">No Schedules Found</h3>
                <p className="text-sm text-center max-w-md mb-4">
                  {searchQuery || statusFilter !== "all"
                    ? "No schedules match your search criteria."
                    : "No scheduled tasks have been created yet. Click 'New Schedule' to create one."}
                </p>
              </div>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
