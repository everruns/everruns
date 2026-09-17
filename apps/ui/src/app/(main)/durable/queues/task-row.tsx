"use client";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { TableCell, TableRow } from "@/components/ui/table";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { AlertTriangle, CheckCircle, Clock, XCircle, Activity, ExternalLink } from "lucide-react";
import Link from "next/link";
import { EntityIdentity } from "@/components/ui/entity-identity";
import { formatDistanceToNow } from "@/lib/formatting";
import { getTaskStatusBadgeVariant } from "@/lib/status-utils";
import type { DurableTask, TaskStatus } from "@/lib/api/types";

export function getTaskStatusIcon(status: TaskStatus) {
  switch (status) {
    case "completed":
      return <CheckCircle className="h-4 w-4 text-success" />;
    case "claimed":
      return <Activity className="h-4 w-4 text-info animate-pulse" />;
    case "failed":
      return <XCircle className="h-4 w-4 text-destructive" />;
    case "dead":
      return <AlertTriangle className="h-4 w-4 text-destructive" />;
    case "cancelled":
      return <AlertTriangle className="h-4 w-4 text-warning" />;
    default:
      return <Clock className="h-4 w-4 text-muted-foreground" />;
  }
}

export function TaskRow({ task }: { task: DurableTask }) {
  return (
    <TableRow>
      <TableCell>
        <div className="flex items-center gap-2">
          {getTaskStatusIcon(task.status)}
          <div>
            <p className="font-medium">
              <EntityIdentity value={task.id}>{task.activity_type}</EntityIdentity>
            </p>
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
              <TooltipTrigger className="text-sm text-destructive max-w-[150px] truncate block">
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
          {task.workflow_id ? (
            <Button
              variant="ghost"
              size="sm"
              aria-label={`View workflow ${task.workflow_id}`}
              render={<Link href={`/durable/workflows/${task.workflow_id}`} />}
            >
              <ExternalLink className="h-3 w-3" />
            </Button>
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
