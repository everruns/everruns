"use client";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { TableCell, TableRow } from "@/components/ui/table";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { RotateCcw, Trash2, ExternalLink } from "lucide-react";
import Link from "next/link";
import { formatDistanceToNow } from "@/lib/formatting";
import type { DlqEntry } from "@/lib/api/types";

export function DlqRow({
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
