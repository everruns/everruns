// Custom React Flow node components for trajectory visualization.
// Designed to be compact for rendering thousands of nodes.

import { memo } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import { User, Bot, Brain, Wrench, Play, CheckCircle2, XCircle, Clock } from "lucide-react";
import type {
  UserMessageNodeData,
  AgentMessageNodeData,
  ReasoningNodeData,
  ToolGroupNodeData,
  TurnNodeData,
} from "./trajectory-utils";

function formatDuration(ms?: number): string {
  if (ms == null) return "";
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

// --- Turn boundary nodes ---

export const TurnStartNode = memo(function TurnStartNode({
  data,
}: NodeProps & { data: TurnNodeData }) {
  const d = data as TurnNodeData;
  return (
    <div className="flex items-center gap-2 px-3 py-1.5 rounded-full bg-muted border border-border text-xs font-medium text-muted-foreground">
      <Play className="w-3 h-3" />
      <span>{d.label}</span>
      {d.iterations > 0 && (
        <span className="text-muted-foreground/60">
          {d.iterations} iter{d.iterations !== 1 ? "s" : ""}
        </span>
      )}
      {d.durationMs != null && (
        <span className="text-muted-foreground/60">
          <Clock className="w-2.5 h-2.5 inline mr-0.5" />
          {formatDuration(d.durationMs)}
        </span>
      )}
      <Handle type="target" position={Position.Top} className="!w-1.5 !h-1.5 !bg-border" />
      <Handle type="source" position={Position.Bottom} className="!w-1.5 !h-1.5 !bg-border" />
    </div>
  );
});

export const TurnEndNode = memo(function TurnEndNode({ data }: NodeProps & { data: TurnNodeData }) {
  const d = data as TurnNodeData;
  const failed = d.failed;
  return (
    <div
      className={`flex items-center gap-1.5 px-3 py-1 rounded-full text-xs font-medium border ${
        failed
          ? "bg-red-50 border-red-200 text-red-700 dark:bg-red-950 dark:border-red-800 dark:text-red-300"
          : "bg-muted border-border text-muted-foreground"
      }`}
    >
      {failed ? <XCircle className="w-3 h-3" /> : <CheckCircle2 className="w-3 h-3" />}
      <span>{d.label}</span>
      <Handle type="target" position={Position.Top} className="!w-1.5 !h-1.5 !bg-border" />
      <Handle type="source" position={Position.Bottom} className="!w-1.5 !h-1.5 !bg-border" />
    </div>
  );
});

// --- User message node ---

export const UserMessageNode = memo(function UserMessageNode({
  data,
}: NodeProps & { data: UserMessageNodeData }) {
  const d = data as UserMessageNodeData;
  return (
    <div className="max-w-[320px] rounded-lg border border-blue-200 bg-blue-50 px-3 py-2 dark:border-blue-800 dark:bg-blue-950">
      <div className="flex items-center gap-1.5 text-xs font-medium text-blue-700 dark:text-blue-300 mb-1">
        <User className="w-3 h-3" />
        User
      </div>
      {d.preview && (
        <p className="text-xs text-blue-900/80 dark:text-blue-200/80 line-clamp-2 leading-relaxed">
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
    <div className="max-w-[320px] rounded-lg border border-violet-200 bg-violet-50 px-3 py-2 dark:border-violet-800 dark:bg-violet-950">
      <div className="flex items-center gap-1.5 text-xs font-medium text-violet-700 dark:text-violet-300 mb-1">
        <Bot className="w-3 h-3" />
        Agent Response
      </div>
      {d.preview && (
        <p className="text-xs text-violet-900/80 dark:text-violet-200/80 line-clamp-2 leading-relaxed">
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
    <div className="rounded-lg border border-amber-200 bg-amber-50 px-3 py-1.5 dark:border-amber-800 dark:bg-amber-950">
      <div className="flex items-center gap-2 text-xs">
        <Brain className="w-3 h-3 text-amber-600 dark:text-amber-400" />
        <span className="font-medium text-amber-700 dark:text-amber-300">{d.label}</span>
        {d.toolCallCount > 0 && (
          <span className="text-amber-600/70 dark:text-amber-400/70">
            → {d.toolCallCount} tool{d.toolCallCount !== 1 ? "s" : ""}
          </span>
        )}
        {d.durationMs != null && (
          <span className="text-amber-600/50 dark:text-amber-400/50">
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
  const hasErrors = d.errorCount > 0;
  return (
    <div
      className={`max-w-[320px] rounded-lg border px-3 py-2 ${
        hasErrors
          ? "border-red-200 bg-red-50 dark:border-red-800 dark:bg-red-950"
          : "border-emerald-200 bg-emerald-50 dark:border-emerald-800 dark:bg-emerald-950"
      }`}
    >
      <div className="flex items-center gap-2 text-xs mb-1">
        <Wrench
          className={`w-3 h-3 ${hasErrors ? "text-red-600 dark:text-red-400" : "text-emerald-600 dark:text-emerald-400"}`}
        />
        <span
          className={`font-medium ${hasErrors ? "text-red-700 dark:text-red-300" : "text-emerald-700 dark:text-emerald-300"}`}
        >
          {d.label}
        </span>
        {d.durationMs != null && (
          <span className="text-muted-foreground/50">{formatDuration(d.durationMs)}</span>
        )}
      </div>
      <div className="flex flex-wrap gap-1">
        {d.tools.map((tool) => (
          <span
            key={tool.id}
            className={`inline-flex items-center gap-0.5 text-[10px] px-1.5 py-0.5 rounded-md ${
              tool.status === "error" || !tool.success
                ? "bg-red-100 text-red-700 dark:bg-red-900 dark:text-red-300"
                : "bg-emerald-100 text-emerald-700 dark:bg-emerald-900 dark:text-emerald-300"
            }`}
          >
            {tool.success ? (
              <CheckCircle2 className="w-2.5 h-2.5" />
            ) : (
              <XCircle className="w-2.5 h-2.5" />
            )}
            {tool.name}
          </span>
        ))}
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

// Export node type mapping for React Flow
export const trajectoryNodeTypes = {
  userMessage: UserMessageNode,
  agentMessage: AgentMessageNode,
  reasoning: ReasoningNode,
  toolGroup: ToolGroupNode,
  turnStart: TurnStartNode,
  turnEnd: TurnEndNode,
};
