// Transforms session events into React Flow nodes and edges for trajectory visualization.
//
// Layout: each turn is a group node (box) containing child nodes.
// Children use parentId + relative positioning. Edges use bezier curves.
//
// Performance design for large trajectories (1000+ turns):
// - buildTurns() is O(n) single-pass over events
// - buildTrajectory() is O(n) over turns
// - Callers should gate recomputation on STRUCTURAL_EVENT_TYPES count,
//   NOT on every SSE delta event (which fires 10-30x per second during streaming).

import type { Node, Edge } from "@xyflow/react";
import type { Event, ToolCompletedData } from "@/lib/api/types";
import { getTextFromContent, getEventData } from "@/lib/api/types";

// --- Node data types ---

export interface UserMessageNodeData {
  label: string;
  preview: string;
  eventId: string;
  timestamp: string;
}

export interface AgentMessageNodeData {
  label: string;
  preview: string;
  eventId: string;
  timestamp: string;
  hasToolCalls: boolean;
  /** Execution phase: "in_progress" or "completed" */
  phase?: string;
}

export interface ReasoningNodeData {
  label: string;
  iteration: number;
  hasToolCalls: boolean;
  toolCallCount: number;
  durationMs?: number;
  eventId: string;
  timestamp: string;
}

export interface ToolGroupNodeData {
  label: string;
  tools: Array<{
    id: string;
    name: string;
    /** Human-readable display name for UI rendering */
    displayName?: string;
    success: boolean;
    status: string;
    durationMs?: number;
  }>;
  successCount: number;
  errorCount: number;
  durationMs?: number;
  eventId: string;
  timestamp: string;
}

export interface TurnGroupNodeData {
  label: string;
  turnId: string;
  turnNumber: number;
  iterations: number;
  durationMs?: number;
  failed: boolean;
  errorMessage?: string;
  timestamp: string;
}

// Union of all node data types
export type TrajectoryNodeData =
  | UserMessageNodeData
  | AgentMessageNodeData
  | ReasoningNodeData
  | ToolGroupNodeData
  | TurnGroupNodeData;

// Node type identifiers
export type TrajectoryNodeType =
  | "userMessage"
  | "agentMessage"
  | "reasoning"
  | "toolGroup"
  | "turnGroup";

// --- Trajectory stats ---

export interface TrajectoryStats {
  turnCount: number;
  completedTurns: number;
  failedTurns: number;
  totalIterations: number;
  totalToolCalls: number;
  totalToolErrors: number;
  totalDurationMs: number;
}

// --- Event types that change the trajectory structure ---
export const STRUCTURAL_EVENT_TYPES = new Set([
  "turn.started",
  "turn.completed",
  "turn.failed",
  "turn.cancelled",
  "input.message",
  "output.message.completed",
  "reason.completed",
  "act.started",
  "act.completed",
  "tool.completed",
]);

// --- Layout constants ---
const GROUP_PAD_X = 16;
const GROUP_PAD_TOP = 44; // space for turn header inside group
const GROUP_PAD_BOTTOM = 16;
const GROUP_WIDTH = 340;
const CHILD_X = GROUP_PAD_X; // child x offset inside group
const CHILD_SPACING_COMPACT = 44; // reasoning
const CHILD_SPACING_CONTENT = 80; // user msg, agent msg, tools
const GROUP_GAP = 40; // vertical gap between turn groups

function truncate(text: string, maxLen: number): string {
  if (text.length <= maxLen) return text;
  return text.slice(0, maxLen) + "…";
}

// --- Build trajectory from events ---

export interface TurnAccumulator {
  turnId: string;
  startTs: string;
  inputMessageId?: string;
  userMessage?: { eventId: string; text: string; ts: string; messageId?: string };
  iterations: IterationAccumulator[];
  agentMessage?: {
    eventId: string;
    text: string;
    ts: string;
    hasToolCalls: boolean;
    phase?: string;
  };
  completed: boolean;
  failed: boolean;
  errorMessage?: string;
  durationMs?: number;
  iterationCount?: number;
}

export interface IterationAccumulator {
  reasoning?: {
    eventId: string;
    ts: string;
    hasToolCalls: boolean;
    toolCallCount: number;
    durationMs?: number;
  };
  toolGroup?: {
    eventId: string;
    ts: string;
    tools: Array<{
      id: string;
      name: string;
      success: boolean;
      status: string;
      durationMs?: number;
    }>;
    successCount: number;
    errorCount: number;
    durationMs?: number;
  };
}

export function buildTurns(events: Event[]): TurnAccumulator[] {
  const turns: TurnAccumulator[] = [];
  let currentTurn: TurnAccumulator | null = null;
  let currentIteration: IterationAccumulator | null = null;

  const toolResults = new Map<string, ToolCompletedData>();
  for (const event of events) {
    const tcData = getEventData(event, "tool.completed");
    if (tcData) {
      toolResults.set(tcData.tool_call_id, tcData);
    }
  }

  for (const event of events) {
    if (!STRUCTURAL_EVENT_TYPES.has(event.type)) continue;

    switch (event.type) {
      case "turn.started": {
        const data = getEventData(event, "turn.started");
        if (!data) break;

        // Check if previous turn is an orphan (input.message arrived before turn.started).
        // Adopt it by updating its turnId instead of creating a duplicate turn.
        const lastTurn = turns.length > 0 ? turns[turns.length - 1] : null;
        const isOrphan =
          lastTurn &&
          !lastTurn.completed &&
          lastTurn.turnId.startsWith("orphan-") &&
          lastTurn.userMessage?.messageId === data.input_message_id;

        if (isOrphan && lastTurn) {
          lastTurn.turnId = data.turn_id;
          lastTurn.inputMessageId = data.input_message_id;
          currentTurn = lastTurn;
        } else {
          currentTurn = {
            turnId: data.turn_id,
            startTs: event.ts,
            inputMessageId: data.input_message_id,
            iterations: [],
            completed: false,
            failed: false,
          };
          turns.push(currentTurn);
        }
        currentIteration = null;
        break;
      }

      case "input.message": {
        const data = getEventData(event, "input.message");
        if (!data) break;
        const text = getTextFromContent(data.message?.content || []);
        const messageId = data.message?.id;
        if (currentTurn) {
          currentTurn.userMessage = { eventId: event.id, text, ts: event.ts, messageId };
        } else {
          const orphanTurn: TurnAccumulator = {
            turnId: `orphan-${event.id}`,
            startTs: event.ts,
            userMessage: { eventId: event.id, text, ts: event.ts, messageId },
            iterations: [],
            completed: false,
            failed: false,
          };
          turns.push(orphanTurn);
          currentTurn = orphanTurn;
        }
        break;
      }

      case "reason.completed": {
        if (!currentTurn) break;
        const data = getEventData(event, "reason.completed");
        if (!data) break;
        currentIteration = {
          reasoning: {
            eventId: event.id,
            ts: event.ts,
            hasToolCalls: data.has_tool_calls,
            toolCallCount: data.tool_call_count,
            durationMs: data.duration_ms,
          },
        };
        currentTurn.iterations.push(currentIteration);
        break;
      }

      case "act.started": {
        if (!currentTurn || !currentIteration) break;
        const data = getEventData(event, "act.started");
        if (!data) break;
        currentIteration.toolGroup = {
          eventId: event.id,
          ts: event.ts,
          tools: data.tool_calls.map((tc) => {
            const result = toolResults.get(tc.id);
            return {
              id: tc.id,
              name: tc.name,
              displayName: tc.display_name ?? result?.display_name,
              success: result?.success ?? true,
              status: result?.status ?? "pending",
              durationMs: result?.duration_ms,
            };
          }),
          successCount: 0,
          errorCount: 0,
        };
        break;
      }

      case "act.completed": {
        if (!currentTurn || !currentIteration?.toolGroup) break;
        const data = getEventData(event, "act.completed");
        if (!data) break;
        currentIteration.toolGroup.successCount = data.success_count;
        currentIteration.toolGroup.errorCount = data.error_count;
        currentIteration.toolGroup.durationMs = data.duration_ms;
        break;
      }

      case "output.message.completed": {
        if (!currentTurn) break;
        const data = getEventData(event, "output.message.completed");
        if (!data) break;
        const text = getTextFromContent(data.message?.content || []);
        const hasToolCalls = data.message?.content?.some((p) => p.type === "tool_call") ?? false;
        currentTurn.agentMessage = {
          eventId: event.id,
          text,
          ts: event.ts,
          hasToolCalls,
          phase: data.message?.phase,
        };
        break;
      }

      case "turn.completed": {
        if (!currentTurn) break;
        const data = getEventData(event, "turn.completed");
        if (!data) break;
        currentTurn.completed = true;
        currentTurn.durationMs = data.duration_ms;
        currentTurn.iterationCount = data.iterations;
        currentTurn = null;
        currentIteration = null;
        break;
      }

      case "turn.failed": {
        if (!currentTurn) break;
        const data = getEventData(event, "turn.failed");
        if (!data) break;
        currentTurn.completed = true;
        currentTurn.failed = true;
        currentTurn.errorMessage = data.error;
        currentTurn = null;
        currentIteration = null;
        break;
      }
    }
  }

  return turns;
}

function computeStats(turns: TurnAccumulator[]): TrajectoryStats {
  let completedTurns = 0;
  let failedTurns = 0;
  let totalIterations = 0;
  let totalToolCalls = 0;
  let totalToolErrors = 0;
  let totalDurationMs = 0;

  for (const turn of turns) {
    if (turn.completed) completedTurns++;
    if (turn.failed) failedTurns++;
    totalIterations += turn.iterationCount ?? turn.iterations.length;
    if (turn.durationMs) totalDurationMs += turn.durationMs;

    for (const iter of turn.iterations) {
      if (iter.toolGroup) {
        totalToolCalls += iter.toolGroup.tools.length;
        totalToolErrors += iter.toolGroup.errorCount;
      }
    }
  }

  return {
    turnCount: turns.length,
    completedTurns,
    failedTurns,
    totalIterations,
    totalToolCalls,
    totalToolErrors,
    totalDurationMs,
  };
}

/**
 * Convert structured turns into React Flow nodes and edges.
 *
 * Each turn is a group node containing child nodes via parentId.
 * Edges connect children sequentially within a turn,
 * and from the last child of one turn to the first child of the next.
 */
export function buildTrajectory(events: Event[]): {
  nodes: Node[];
  edges: Edge[];
  stats: TrajectoryStats;
} {
  const turns = buildTurns(events);
  const stats = computeStats(turns);
  const nodes: Node[] = [];
  const edges: Edge[] = [];
  let groupY = 0; // absolute Y for group placement
  let prevChildId: string | null = null; // last child of previous turn

  function addEdge(source: string, target: string) {
    edges.push({
      id: `e-${source}-${target}`,
      source,
      target,
    });
  }

  for (let turnIdx = 0; turnIdx < turns.length; turnIdx++) {
    const turn = turns[turnIdx];
    const groupId = `turn-${turnIdx}-group`;
    let childY = GROUP_PAD_TOP; // relative Y within group
    let firstChildId: string | null = null;
    let lastChildId: string | null = null;

    function addChild(id: string, type: TrajectoryNodeType, data: TrajectoryNodeData) {
      // TrajectoryNodeData variants are plain objects with string keys.
      // React Flow Node requires Record<string, unknown>; TS interfaces lack
      // implicit index signatures, so a single narrowing cast is needed here.
      nodes.push({
        id,
        type,
        position: { x: CHILD_X, y: childY },
        parentId: groupId,
        extent: "parent" as const,
        data: data as TrajectoryNodeData & Record<string, unknown>,
      });
      if (!firstChildId) firstChildId = id;
      if (lastChildId) addEdge(lastChildId, id);
      lastChildId = id;
    }

    // User message
    if (turn.userMessage) {
      const umId = `turn-${turnIdx}-user`;
      addChild(umId, "userMessage", {
        label: "User",
        preview: truncate(turn.userMessage.text, 100),
        eventId: turn.userMessage.eventId,
        timestamp: turn.userMessage.ts,
      } as UserMessageNodeData);
      childY += CHILD_SPACING_CONTENT;
    }

    // Iterations
    for (let iterIdx = 0; iterIdx < turn.iterations.length; iterIdx++) {
      const iter = turn.iterations[iterIdx];

      if (iter.reasoning) {
        const rId = `turn-${turnIdx}-reason-${iterIdx}`;
        addChild(rId, "reasoning", {
          label: `Reasoning ${iterIdx + 1}`,
          iteration: iterIdx + 1,
          hasToolCalls: iter.reasoning.hasToolCalls,
          toolCallCount: iter.reasoning.toolCallCount,
          durationMs: iter.reasoning.durationMs,
          eventId: iter.reasoning.eventId,
          timestamp: iter.reasoning.ts,
        } as ReasoningNodeData);
        childY += CHILD_SPACING_COMPACT;
      }

      if (iter.toolGroup && iter.toolGroup.tools.length > 0) {
        const tgId = `turn-${turnIdx}-tools-${iterIdx}`;
        addChild(tgId, "toolGroup", {
          label: `Tools (${iter.toolGroup.tools.length})`,
          tools: iter.toolGroup.tools,
          successCount: iter.toolGroup.successCount,
          errorCount: iter.toolGroup.errorCount,
          durationMs: iter.toolGroup.durationMs,
          eventId: iter.toolGroup.eventId,
          timestamp: iter.toolGroup.ts,
        } as ToolGroupNodeData);
        childY += CHILD_SPACING_CONTENT + Math.max(0, (iter.toolGroup.tools.length - 3) * 20);
      }
    }

    // Agent message
    if (turn.agentMessage) {
      const amId = `turn-${turnIdx}-agent`;
      addChild(amId, "agentMessage", {
        label: "Agent",
        preview: truncate(turn.agentMessage.text, 100),
        eventId: turn.agentMessage.eventId,
        timestamp: turn.agentMessage.ts,
        hasToolCalls: turn.agentMessage.hasToolCalls,
        phase: turn.agentMessage.phase,
      } as AgentMessageNodeData);
      childY += CHILD_SPACING_CONTENT;
    }

    // Calculate group height from accumulated child positions
    const groupHeight = childY - CHILD_SPACING_CONTENT + CHILD_SPACING_COMPACT + GROUP_PAD_BOTTOM;

    // Create turn group node
    nodes.push({
      id: groupId,
      type: "turnGroup",
      position: { x: 0, y: groupY },
      data: {
        label: `Turn ${turnIdx + 1}`,
        turnId: turn.turnId,
        turnNumber: turnIdx + 1,
        iterations: turn.iterationCount ?? turn.iterations.length,
        durationMs: turn.durationMs,
        failed: turn.failed,
        errorMessage: turn.errorMessage,
        timestamp: turn.startTs,
      } as TurnGroupNodeData & Record<string, unknown>, // single cast: TS interface -> Record
      style: {
        width: GROUP_WIDTH,
        height: Math.max(groupHeight, GROUP_PAD_TOP + 40),
      },
    });

    // Edge from previous turn's last child to this turn's first child
    if (prevChildId && firstChildId) {
      addEdge(prevChildId, firstChildId);
    }
    prevChildId = lastChildId;

    groupY += Math.max(groupHeight, GROUP_PAD_TOP + 40) + GROUP_GAP;
  }

  return { nodes, edges, stats };
}

export function countStructuralEvents(events: Event[] | undefined): number {
  if (!events) return 0;
  let count = 0;
  for (const e of events) {
    if (STRUCTURAL_EVENT_TYPES.has(e.type)) count++;
  }
  return count;
}
