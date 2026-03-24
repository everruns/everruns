"use client";

/**
 * Decisions:
 * - Group tool-only assistant steps into a single activity block so the transcript reads like progress, not raw logs.
 * - Shell calls stay standalone in transcript order; they should not be wrapped in the grouped activity card.
 * - Error emphasis relies on iconography and copy, not red box styling around the activity row.
 * - Keep motion CSS-only: fade/slide in, status transitions, and collapsible output without adding another runtime dependency.
 * - Match the Slate system: sharp corners, grayscale surfaces, and gold border accents for active work.
 */

import { useMemo } from "react";
import type { ToolCompletedData, ToolProgressData } from "@/lib/api/types";
import type { ToolOutputStreams } from "@/app/(main)/sessions/[sessionId]/session-context";
import { isWriteTodosTool } from "@/lib/tool-registry";
import { BashToolCallCard } from "./bash-tool-call-card";
import { GroupedActivityCard } from "./grouped-activity-card";
import { ReadFileToolCallCard } from "./read-file-tool-call-card";
import { TodoListRenderer } from "./todo-list-renderer";
import { buildActivitySegments } from "./tool-activity-utils";
import type { ToolCallContent } from "./tool-call-utils";
import { WriteFileToolCallCard } from "./write-file-tool-call-card";

interface ToolActivityGroupProps {
  toolCalls: ToolCallContent[];
  toolResultsMap: Map<string, ToolCompletedData>;
  toolProgressMap?: Map<string, ToolProgressData>;
  toolOutputMap?: Map<string, ToolOutputStreams>;
  mode?: "server" | "client";
}

export function ToolActivityGroup({
  toolCalls,
  toolResultsMap,
  toolProgressMap,
  toolOutputMap,
  mode = "server",
}: ToolActivityGroupProps) {
  const todoToolCalls = toolCalls.filter((toolCall) => isWriteTodosTool(toolCall.name));
  const activityToolCalls = toolCalls.filter((toolCall) => !isWriteTodosTool(toolCall.name));
  const activitySegments = useMemo(
    () => buildActivitySegments(activityToolCalls, mode),
    [activityToolCalls, mode],
  );

  if (toolCalls.length === 0) return null;

  return (
    <div className="space-y-3">
      {activitySegments.map((segment, index) => {
        if (segment.type === "shell") {
          return (
            <BashToolCallCard
              key={segment.toolCall.id}
              toolCall={segment.toolCall}
              toolResult={toolResultsMap.get(segment.toolCall.id)}
              streamedOutput={toolOutputMap?.get(segment.toolCall.id)}
            />
          );
        }

        if (segment.type === "read_file") {
          return (
            <ReadFileToolCallCard
              key={segment.toolCall.id}
              toolCall={segment.toolCall}
              toolResult={toolResultsMap.get(segment.toolCall.id)}
            />
          );
        }

        if (segment.type === "write_file") {
          return (
            <WriteFileToolCallCard
              key={segment.toolCall.id}
              toolCall={segment.toolCall}
              toolResult={toolResultsMap.get(segment.toolCall.id)}
            />
          );
        }

        return (
          <GroupedActivityCard
            key={`${segment.toolCalls[0]?.id ?? "group"}-${index}`}
            toolCalls={segment.toolCalls}
            toolResultsMap={toolResultsMap}
            toolProgressMap={toolProgressMap}
            mode={mode}
          />
        );
      })}

      {todoToolCalls.map((toolCall) => {
        const toolResult = toolResultsMap.get(toolCall.id);
        return (
          <TodoListRenderer
            key={toolCall.id}
            arguments={toolCall.arguments}
            result={toolResult?.result}
            isExecuting={!toolResult}
            error={toolResult?.error}
          />
        );
      })}
    </div>
  );
}
