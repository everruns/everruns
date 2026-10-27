"use client";

import { useLocale as useFormattingLocale } from "@/providers/locale-provider";
import { useUiTranslations } from "@/providers/locale-provider";
import Link from "next/link";
import { AlertTriangle } from "lucide-react";
import type { SandboxFleetItem } from "@/lib/api/sandboxes";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { formatRelativeTime } from "@/lib/formatting";
import { cn } from "@/lib/utils";
import { SandboxStateBadge, attentionTitle, formatAge, providerLabel } from "./sandbox-display";

/** Name of a Sandbox in lists: its Session, else what is left of it. */
export function sandboxTitle(sandbox: SandboxFleetItem): string {
  return sandbox.session_title || (sandbox.session_id ? "Untitled session" : "Deleted session");
}

export function SandboxFleetTable({
  items,
  selectedId,
  onSelect,
}: {
  items: SandboxFleetItem[];
  selectedId: string | null;
  onSelect: (id: string) => void;
}) {
  const { displayLocale } = useFormattingLocale();
  const ui = useUiTranslations();
  return (
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead>{ui("State")}</TableHead>
          <TableHead>{ui("Session")}</TableHead>
          <TableHead>{ui("Agent")}</TableHead>
          <TableHead>{ui("Provider")}</TableHead>
          <TableHead>{ui("Template")}</TableHead>
          <TableHead className="text-right">
            <Tooltip>
              <TooltipTrigger className="cursor-help">{ui("Gen")}</TooltipTrigger>
              <TooltipContent>
                {ui(
                  "How many provider resources this Sandbox has had. Above 1 means it was lost and rebuilt.",
                )}
              </TooltipContent>
            </Tooltip>
          </TableHead>
          <TableHead className="text-right">{ui("Checkpoints")}</TableHead>
          <TableHead>{ui("Last active")}</TableHead>
          <TableHead className="text-right">{ui("Age")}</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {items.map((sandbox) => {
          const selected = sandbox.id === selectedId;
          return (
            <TableRow
              key={sandbox.id}
              data-state={selected ? "selected" : undefined}
              className={cn("cursor-pointer", selected && "bg-muted/60")}
              onClick={() => onSelect(sandbox.id)}
            >
              <TableCell>
                <div className="flex items-center gap-2">
                  <SandboxStateBadge state={sandbox.state} />
                  {sandbox.attention.length > 0 ? (
                    <Tooltip>
                      <TooltipTrigger
                        aria-label={sandbox.attention.map(attentionTitle).join(", ")}
                        className="text-destructive"
                      >
                        <AlertTriangle className="size-3.5" />
                      </TooltipTrigger>
                      <TooltipContent>
                        {sandbox.attention.map(attentionTitle).join(". ")}
                      </TooltipContent>
                    </Tooltip>
                  ) : null}
                </div>
              </TableCell>
              <TableCell className="max-w-72">
                {/* The row opens the drawer; the button keeps it keyboard-reachable. */}
                <button
                  type="button"
                  onClick={(event) => {
                    event.stopPropagation();
                    onSelect(sandbox.id);
                  }}
                  className={cn(
                    "block max-w-full truncate text-left font-medium hover:underline",
                    !sandbox.session_title && "text-muted-foreground",
                  )}
                >
                  {sandboxTitle(sandbox)}
                </button>
              </TableCell>
              <TableCell>
                {sandbox.agent_id ? (
                  <Link
                    href={`/agents/${sandbox.agent_id}`}
                    className="hover:underline"
                    onClick={(event) => event.stopPropagation()}
                  >
                    {sandbox.agent_name ?? sandbox.agent_id}
                  </Link>
                ) : (
                  <span className="text-muted-foreground">{ui("None")}</span>
                )}
              </TableCell>
              <TableCell>{providerLabel(sandbox.provider)}</TableCell>
              <TableCell className="font-mono text-xs text-muted-foreground">
                {sandbox.template
                  ? ui("{value0} r{value1}", {
                      value0: sandbox.template.display_name ?? "Template",
                      value1: sandbox.template.revision ?? "?",
                    })
                  : ui("No template")}
              </TableCell>
              <TableCell
                className={cn(
                  "text-right tabular-nums",
                  sandbox.generation > 1 && "font-medium text-warning",
                )}
              >
                {sandbox.generation}
              </TableCell>
              <TableCell className="text-right tabular-nums">{sandbox.checkpoint_count}</TableCell>
              <TableCell className="text-muted-foreground">
                {formatRelativeTime(sandbox.last_activity_at ?? sandbox.updated_at, displayLocale)}
              </TableCell>
              <TableCell className="text-right tabular-nums text-muted-foreground">
                {formatAge(sandbox.created_at, sandbox.deleted_at ?? null)}
              </TableCell>
            </TableRow>
          );
        })}
      </TableBody>
    </Table>
  );
}
