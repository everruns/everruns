import type {
  WorkflowStatus,
  TaskStatus,
  WorkerStatus,
  ScheduleExecutionStatus,
  CapabilityStatus,
} from "@/lib/api/types";

type BadgeVariant = "default" | "secondary" | "destructive" | "outline";

export function getWorkflowStatusBadgeVariant(status: WorkflowStatus): BadgeVariant {
  switch (status) {
    case "completed":
      return "default";
    case "running":
      return "secondary";
    case "failed":
      return "destructive";
    case "cancelled":
    case "pending":
      return "outline";
    default:
      return "outline";
  }
}

export function getTaskStatusBadgeVariant(status: TaskStatus): BadgeVariant {
  switch (status) {
    case "completed":
      return "default";
    case "claimed":
      return "secondary";
    case "failed":
    case "dead":
      return "destructive";
    default:
      return "outline";
  }
}

export function getWorkerStatusBadgeVariant(status: WorkerStatus): BadgeVariant {
  switch (status) {
    case "active":
      return "default";
    case "draining":
      return "secondary";
    case "stopped":
      return "destructive";
    case "stale":
      return "outline";
    default:
      return "outline";
  }
}

export function getHealthBadgeVariant(status: string): BadgeVariant {
  switch (status) {
    case "healthy":
      return "default";
    case "degraded":
      return "secondary";
    case "unhealthy":
      return "destructive";
    default:
      return "outline";
  }
}

export function getScheduleExecutionBadgeVariant(status: ScheduleExecutionStatus): BadgeVariant {
  switch (status) {
    case "completed":
      return "default";
    case "running":
      return "secondary";
    case "failed":
      return "destructive";
    case "skipped":
    case "pending":
      return "outline";
    default:
      return "outline";
  }
}

export function getCapabilityStatusBadgeVariant(
  status: CapabilityStatus,
): "default" | "secondary" | "outline" {
  switch (status) {
    case "available":
      return "default";
    case "coming_soon":
      return "secondary";
    case "deprecated":
      return "outline";
  }
}
