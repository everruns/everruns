"use client";

// Tools of one org MCP server, with a per-tool choice of whether it asks before
// it runs (knowledge/integrations/mcp-servers.md, "Tool risk labels").
//
// Decisions:
// - A suggestion is shown, never applied: "Use suggestion" sets the label, which
//   is the person confirming it. The server drops the suggestion once a label
//   is set.
// - The choice is a three-way radio group rather than a select, so every option
//   and the current one are visible at a glance in a list of many tools.

import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  useMcpServerTools,
  useSetMcpToolLabel,
  useSuggestMcpToolLabels,
} from "@/hooks/use-mcp-servers";
import type { McpServer, McpServerTool, McpToolLabel } from "@/lib/api/types";
import { cn } from "@/lib/utils";
import { Sparkles } from "lucide-react";

type Choice = McpToolLabel | "default";

const CHOICES: Array<{ value: Choice; label: string }> = [
  { value: "default", label: "Default" },
  { value: "read_only", label: "Read only" },
  { value: "changes", label: "Changes things" },
];

const SUGGESTION_TEXT: Record<McpToolLabel, string> = {
  read_only: "read only",
  changes: "changes things",
};

/** What the server itself says about the tool, in plain words. */
export function describeAnnotations(annotations: McpServerTool["annotations"]): string[] {
  if (!annotations) return [];
  const notes: string[] = [];
  if (annotations.readOnlyHint === true) notes.push("only reads");
  if (annotations.readOnlyHint === false) notes.push("can change things");
  if (annotations.destructiveHint === true) notes.push("can delete or overwrite");
  if (annotations.destructiveHint === false) notes.push("does not delete");
  if (annotations.idempotentHint === true) notes.push("safe to repeat");
  if (annotations.openWorldHint === true) notes.push("reaches other services");
  if (annotations.openWorldHint === false) notes.push("stays inside its own service");
  return notes;
}

function errorText(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : fallback;
}

function ToolRow({
  tool,
  canEdit,
  saving,
  onChoose,
}: {
  tool: McpServerTool;
  canEdit: boolean;
  saving: boolean;
  onChoose: (label: McpToolLabel | null) => void;
}) {
  const current: Choice = tool.label ?? "default";
  const notes = describeAnnotations(tool.annotations);
  const suggestion = tool.label ? null : tool.suggested_label;

  return (
    <li className="space-y-2 border-b py-3 last:border-b-0" data-testid={`tool-${tool.name}`}>
      <div className="min-w-0">
        <div className="font-mono text-sm font-medium break-all">{tool.name}</div>
        {tool.title && tool.title !== tool.name && <div className="text-sm">{tool.title}</div>}
        {tool.description && (
          <p className="text-xs text-muted-foreground line-clamp-3">{tool.description}</p>
        )}
        <p className="text-xs text-muted-foreground">
          {notes.length > 0
            ? `The server says it ${notes.join(", ")}.`
            : "The server says nothing about what this tool does."}
        </p>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <div
          role="radiogroup"
          aria-label={`Approval for ${tool.name}`}
          className="inline-flex border"
        >
          {CHOICES.map((choice) => {
            const selected = current === choice.value;
            return (
              <button
                key={choice.value}
                type="button"
                role="radio"
                aria-checked={selected}
                disabled={!canEdit || saving}
                onClick={() => {
                  if (!selected) onChoose(choice.value === "default" ? null : choice.value);
                }}
                className={cn(
                  "px-2.5 py-1 text-xs disabled:opacity-60",
                  selected ? "bg-primary text-primary-foreground" : "hover:bg-muted",
                )}
              >
                {choice.label}
              </button>
            );
          })}
        </div>
        {suggestion && (
          <>
            <Badge variant="outline">Suggested: {SUGGESTION_TEXT[suggestion]}</Badge>
            {canEdit && (
              <Button
                variant="ghost"
                size="sm"
                disabled={saving}
                onClick={() => onChoose(suggestion)}
              >
                Use suggestion
              </Button>
            )}
          </>
        )}
      </div>
    </li>
  );
}

export function McpToolLabelsDialog({
  server,
  canEdit,
  open,
  onOpenChange,
}: {
  server: McpServer;
  /** Whether the viewer may change labels (manage policy, server not archived). */
  canEdit: boolean;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const tools = useMcpServerTools(open ? server.id : undefined);
  const setLabel = useSetMcpToolLabel(server.id);
  const suggest = useSuggestMcpToolLabels(server.id);
  const savingTool = setLabel.isPending ? setLabel.variables?.toolName : undefined;
  const list = tools.data ?? [];
  const unlabeled = list.filter((tool) => !tool.label).length;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>Tools of {server.name}</DialogTitle>
          <DialogDescription>
            Choose which tools ask before they run. A read-only tool runs without asking. A tool
            that changes things asks first. By default every MCP tool asks.
          </DialogDescription>
        </DialogHeader>

        {canEdit && list.length > 0 && (
          <div className="flex flex-wrap items-center justify-between gap-2">
            <p className="text-xs text-muted-foreground">
              Suggestions only fill in a hint. Nothing changes until you pick a choice.
            </p>
            <Button
              variant="outline"
              size="sm"
              disabled={suggest.isPending || unlabeled === 0}
              onClick={() => suggest.mutate()}
            >
              <Sparkles className="size-4" />
              {suggest.isPending ? "Suggesting…" : "Suggest labels"}
            </Button>
          </div>
        )}
        {suggest.error && (
          <p role="alert" className="text-xs text-destructive">
            {errorText(suggest.error, "Could not suggest labels.")}
          </p>
        )}
        {setLabel.error && (
          <p role="alert" className="text-xs text-destructive">
            {errorText(setLabel.error, "Could not save the choice.")}
          </p>
        )}

        <div className="max-h-[60vh] overflow-y-auto">
          {tools.isLoading ? (
            <div className="space-y-3">
              <Skeleton className="h-12 w-full" />
              <Skeleton className="h-12 w-full" />
            </div>
          ) : tools.error ? (
            <p role="alert" className="text-sm text-destructive">
              {errorText(tools.error, "Could not load the tools.")}
            </p>
          ) : list.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No tools are known for this server yet. They appear once an agent or a person has
              connected to it.
            </p>
          ) : (
            <ul>
              {list.map((tool) => (
                <ToolRow
                  key={tool.name}
                  tool={tool}
                  canEdit={canEdit}
                  saving={savingTool === tool.name}
                  onChoose={(label) => setLabel.mutate({ toolName: tool.name, label })}
                />
              ))}
            </ul>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
