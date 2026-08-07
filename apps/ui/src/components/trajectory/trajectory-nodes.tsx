// Custom React Flow node components for trajectory visualization.
// TurnGroupNode renders as a container box; child nodes render inside it.

import { memo } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import {
  User,
  Bot,
  Brain,
  Wrench,
  CheckCircle2,
  XCircle,
  Clock,
  Loader2,
  ChevronRight,
} from "lucide-react";
import type {
  UserMessageNodeData,
  AgentMessageNodeData,
  ReasoningNodeData,
  ToolGroupNodeData,
  TurnGroupNodeData,
} from "./trajectory-utils";
import { formatToolDetail } from "./trajectory-utils";
import { formatDurationCompact } from "@/lib/formatting";

function formatDuration(ms?: number): string {
  if (ms == null) return "";
  return formatDurationCompact(ms);
}

// --- Turn group node (container box) ---

export const TurnGroupNode = memo(function TurnGroupNode({
  data,
}: NodeProps & { data: TurnGroupNodeData }) {
  const d = data as TurnGroupNodeData;
  const failed = d.failed;
  return (
    <div
      className={`w-full h-full border-2 ${
        failed
          ? "border-red-300 bg-red-50/50 dark:border-red-700 dark:bg-red-950/30"
          : "border-border bg-muted/20 dark:bg-muted/10"
      }`}
    >
      <div
        className={`flex items-center gap-2 px-3 py-2 text-xs font-semibold ${
          failed
            ? "text-red-700 bg-red-100/60 dark:text-red-300 dark:bg-red-900/30"
            : "text-foreground/80 bg-muted/40"
        }`}
      >
        <span>{d.label}</span>
        {d.iterations > 0 && (
          <span className="font-normal text-muted-foreground">
            {d.iterations} iter{d.iterations !== 1 ? "s" : ""}
          </span>
        )}
        {d.durationMs != null && (
          <span className="font-normal text-muted-foreground">
            <Clock className="w-2.5 h-2.5 inline mr-0.5" />
            {formatDuration(d.durationMs)}
          </span>
        )}
        {failed && (
          <span className="ml-auto flex items-center gap-1 text-red-600 dark:text-red-400">
            <XCircle className="w-3 h-3" />
            Failed
          </span>
        )}
      </div>
      {failed && d.errorMessage && (
        <div className="px-3 py-2 text-[11px] leading-relaxed text-red-700/85 dark:text-red-200/85 line-clamp-2">
          {d.errorMessage}
        </div>
      )}
    </div>
  );
});

// --- User message node ---

export const UserMessageNode = memo(function UserMessageNode({
  data,
}: NodeProps & { data: UserMessageNodeData }) {
  const d = data as UserMessageNodeData;
  return (
    <div className="w-[308px] border border-blue-200 bg-blue-50 px-3 py-2 dark:border-blue-800 dark:bg-blue-950">
      <div className="flex items-center gap-1.5 text-xs font-medium text-blue-700 dark:text-blue-300 mb-0.5">
        <User className="w-3 h-3" />
        User
      </div>
      {d.preview && (
        <p className="text-[11px] text-blue-900/80 dark:text-blue-200/80 line-clamp-2 leading-relaxed">
          {d.preview}
        </p>
      )}
      <Handle type="target" position={Position.Top} className="!w-1.5 !h-1.5 !bg-blue-400" />
      <Handle type="source" position={Position.Bottom} className="!w-1.5 !h-1.5 !bg-blue-400" />
    </div>
  );
});

// --- Agent message node ---

export const AgentMessageNode = memo(function AgentMessageNode({
  data,
}: NodeProps & { data: AgentMessageNodeData }) {
  const d = data as AgentMessageNodeData;
  return (
    <div className="w-[308px] border border-violet-200 bg-violet-50 px-3 py-2 dark:border-violet-800 dark:bg-violet-950">
      <div className="flex items-center gap-1.5 text-xs font-medium text-violet-700 dark:text-violet-300 mb-0.5">
        <Bot className="w-3 h-3" />
        Agent
        {d.phase && (
          <span className="ml-auto text-[10px] font-normal opacity-70">
            {d.phase === "in_progress" ? "working" : d.phase}
          </span>
        )}
      </div>
      {d.preview && (
        <p className="text-[11px] text-violet-900/80 dark:text-violet-200/80 line-clamp-2 leading-relaxed">
          {d.preview}
        </p>
      )}
      <Handle type="target" position={Position.Top} className="!w-1.5 !h-1.5 !bg-violet-400" />
      <Handle type="source" position={Position.Bottom} className="!w-1.5 !h-1.5 !bg-violet-400" />
    </div>
  );
});

// --- Reasoning node ---

export const ReasoningNode = memo(function ReasoningNode({
  data,
}: NodeProps & { data: ReasoningNodeData }) {
  const d = data as ReasoningNodeData;
  return (
    <div className="w-[308px] border border-amber-200 bg-amber-50 px-2.5 py-1.5 dark:border-amber-800 dark:bg-amber-950">
      <div className="flex items-center gap-2 text-xs">
        <Brain className="w-3 h-3 text-amber-600 dark:text-amber-400 shrink-0" />
        <span className="font-medium text-amber-700 dark:text-amber-300">{d.label}</span>
        {d.toolCallCount > 0 && (
          <span className="text-amber-600/70 dark:text-amber-400/70">
            → {d.toolCallCount} tool{d.toolCallCount !== 1 ? "s" : ""}
          </span>
        )}
        {d.durationMs != null && (
          <span className="ml-auto text-amber-600/50 dark:text-amber-400/50">
            {formatDuration(d.durationMs)}
          </span>
        )}
      </div>
      <Handle type="target" position={Position.Top} className="!w-1.5 !h-1.5 !bg-amber-400" />
      <Handle type="source" position={Position.Bottom} className="!w-1.5 !h-1.5 !bg-amber-400" />
    </div>
  );
});

// --- Tool group node ---

export const ToolGroupNode = memo(function ToolGroupNode({
  data,
}: NodeProps & { data: ToolGroupNodeData }) {
  const d = data as ToolGroupNodeData;
  const hasErrors =
    d.errorCount > 0 || d.tools.some((tool) => tool.status !== "pending" && !tool.success);
  return (
    <div
      className={`w-[308px] border px-2.5 py-2 ${
        hasErrors
          ? "border-red-200 bg-red-50 dark:border-red-800 dark:bg-red-950"
          : "border-emerald-200 bg-emerald-50 dark:border-emerald-800 dark:bg-emerald-950"
      }`}
    >
      <div className="flex items-center gap-2 text-xs mb-1">
        <Wrench
          className={`w-3 h-3 shrink-0 ${hasErrors ? "text-red-600 dark:text-red-400" : "text-emerald-600 dark:text-emerald-400"}`}
        />
        <span
          className={`font-medium ${hasErrors ? "text-red-700 dark:text-red-300" : "text-emerald-700 dark:text-emerald-300"}`}
        >
          {d.label}
        </span>
        {d.durationMs != null && (
          <span className="ml-auto text-muted-foreground/50">{formatDuration(d.durationMs)}</span>
        )}
      </div>
      <div className="space-y-1">
        {d.tools.map((tool) => {
          const failed = tool.status === "error" || tool.status === "timeout" || !tool.success;
          const pending = tool.status === "pending";
          const input = formatToolDetail(tool.arguments);
          const output = formatToolDetail(tool.error ?? tool.resultText);
          const hasDetails = input.length > 0 || output.length > 0;

          return (
            <details key={tool.id} className="group/tool relative text-[10px] open:z-20">
              <summary
                className={`nodrag flex min-w-0 list-none items-center gap-1 px-1.5 py-1 marker:hidden ${
                  pending
                    ? "bg-muted text-muted-foreground"
                    : failed
                      ? "bg-red-100 text-red-700 dark:bg-red-900 dark:text-red-300"
                      : "bg-emerald-100 text-emerald-700 dark:bg-emerald-900 dark:text-emerald-300"
                } ${hasDetails ? "cursor-pointer" : "cursor-default"}`}
              >
                {pending ? (
                  <Loader2 className="h-2.5 w-2.5 shrink-0 animate-spin" />
                ) : failed ? (
                  <XCircle className="h-2.5 w-2.5 shrink-0" />
                ) : (
                  <CheckCircle2 className="h-2.5 w-2.5 shrink-0" />
                )}
                <span className="min-w-0 flex-1 truncate font-medium" title={tool.label}>
                  {tool.label}
                </span>
                {tool.label !== tool.name && (
                  <span className="max-w-20 truncate font-mono opacity-55">{tool.name}</span>
                )}
                <span className="capitalize opacity-65">{pending ? "running" : tool.status}</span>
                {tool.durationMs != null && (
                  <span className="tabular-nums opacity-65">{formatDuration(tool.durationMs)}</span>
                )}
                {hasDetails && (
                  <ChevronRight className="h-2.5 w-2.5 shrink-0 transition-transform group-open/tool:rotate-90" />
                )}
              </summary>
              {hasDetails && (
                <div className="nodrag nowheel absolute left-0 top-full z-20 mt-1 w-full max-h-40 overflow-auto border border-border bg-popover p-2 text-popover-foreground shadow-lg">
                  {input && (
                    <div>
                      <div className="mb-0.5 font-medium uppercase tracking-wide text-muted-foreground">
                        Input
                      </div>
                      <pre className="whitespace-pre-wrap break-words font-mono text-[9px] leading-relaxed">
                        {input}
                      </pre>
                    </div>
                  )}
                  {output && (
                    <div className={input ? "mt-2" : undefined}>
                      <div
                        className={`mb-0.5 font-medium uppercase tracking-wide ${
                          failed ? "text-red-600 dark:text-red-400" : "text-muted-foreground"
                        }`}
                      >
                        {failed ? "Error" : "Result"}
                      </div>
                      <pre className="whitespace-pre-wrap break-words font-mono text-[9px] leading-relaxed">
                        {output}
                      </pre>
                    </div>
                  )}
                </div>
              )}
            </details>
          );
        })}
      </div>
      <Handle
        type="target"
        position={Position.Top}
        className={`!w-1.5 !h-1.5 ${hasErrors ? "!bg-red-400" : "!bg-emerald-400"}`}
      />
      <Handle
        type="source"
        position={Position.Bottom}
        className={`!w-1.5 !h-1.5 ${hasErrors ? "!bg-red-400" : "!bg-emerald-400"}`}
      />
    </div>
  );
});

export const trajectoryNodeTypes = {
  userMessage: UserMessageNode,
  agentMessage: AgentMessageNode,
  reasoning: ReasoningNode,
  toolGroup: ToolGroupNode,
  turnGroup: TurnGroupNode,
};
