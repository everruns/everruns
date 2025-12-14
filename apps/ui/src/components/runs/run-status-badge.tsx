"use client";

import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import type { RunStatus } from "@/lib/api/types";

const statusConfig: Record<
  RunStatus,
  { label: string; className: string }
> = {
  pending: {
    label: "Pending",
    className: "bg-yellow-100 text-yellow-800 hover:bg-yellow-100",
  },
  running: {
    label: "Running",
    className: "bg-blue-100 text-blue-800 hover:bg-blue-100 animate-pulse",
  },
  completed: {
    label: "Completed",
    className: "bg-green-100 text-green-800 hover:bg-green-100",
  },
  failed: {
    label: "Failed",
    className: "bg-red-100 text-red-800 hover:bg-red-100",
  },
  cancelled: {
    label: "Cancelled",
    className: "bg-gray-100 text-gray-800 hover:bg-gray-100",
  },
};

interface RunStatusBadgeProps {
  status: RunStatus;
  size?: "sm" | "md" | "lg";
}

export function RunStatusBadge({ status, size = "md" }: RunStatusBadgeProps) {
  const config = statusConfig[status];

  const sizeClasses = {
    sm: "text-xs px-1.5 py-0",
    md: "text-xs px-2 py-0.5",
    lg: "text-sm px-3 py-1",
  };

  return (
    <Badge variant="outline" className={cn(config.className, sizeClasses[size])}>
      {config.label}
    </Badge>
  );
}
