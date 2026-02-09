// Transforms session events into React Flow nodes and edges for trajectory visualization.
// Designed for large trajectories (thousands of iterations) by keeping nodes compact.

import type { Node, Edge } from "@xyflow/react";
import type {
  Event,
  InputMessageData,
  OutputMessageCompletedData,
  TurnStartedData,
  TurnCompletedData,
  TurnFailedData,
  ReasonCompletedData,
  ActStartedData,
  ActCompletedData,
  ToolCompletedData,
} from "@/lib/api/types";
import { getTextFromContent } from "@/lib/api/types";

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

export interface TurnNodeData {
  label: string;
  turnId: string;
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
  | TurnNodeData;

// Node type identifiers
export type TrajectoryNodeType =
  | "userMessage"
  | "agentMessage"
  | "reasoning"
  | "toolGroup"
  | "turnStart"
  | "turnEnd";

// --- Layout constants ---
// Different spacing per node type to prevent overlap
const SPACING_MARKER = 50; // turn start/end pills
const SPACING_COMPACT = 55; // reasoning (single-line)
const SPACING_CONTENT = 100; // user message, agent message, tool group (multi-line)
const MAIN_X = 0;

// Truncate text for preview
function truncate(text: string, maxLen: number): string {
  if (text.length <= maxLen) return text;
  return text.slice(0, maxLen) + "…";
}

// --- Build trajectory from events ---

interface TurnAccumulator {
  turnId: string;
  startTs: string;
  inputMessageId?: string;
  // Collected within this turn
  userMessage?: { eventId: string; text: string; ts: string };
  iterations: IterationAccumulator[];
  agentMessage?: { eventId: string; text: string; ts: string; hasToolCalls: boolean };
  completed: boolean;
  failed: boolean;
  errorMessage?: string;
  durationMs?: number;
  iterationCount?: number;
}

interface IterationAccumulator {
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

/**
 * Build structured turns from raw events.
 * Events arrive chronologically; we group them into turns and iterations.
 */
function buildTurns(events: Event[]): TurnAccumulator[] {
  const turns: TurnAccumulator[] = [];
  let currentTurn: TurnAccumulator | null = null;
  let currentIteration: IterationAccumulator | null = null;

  // Map of tool_call_id -> tool completed data (for matching)
  const toolResults = new Map<string, ToolCompletedData>();

  // First pass: collect tool results
  for (const event of events) {
    if (event.type === "tool.completed") {
      const data = event.data as ToolCompletedData;
      toolResults.set(data.tool_call_id, data);
    }
  }

  for (const event of events) {
    switch (event.type) {
      case "turn.started": {
        const data = event.data as TurnStartedData;
        currentTurn = {
          turnId: data.turn_id,
          startTs: event.ts,
          inputMessageId: data.input_message_id,
          iterations: [],
          completed: false,
          failed: false,
        };
        currentIteration = null;
        turns.push(currentTurn);
        break;
      }

      case "input.message": {
        const data = event.data as InputMessageData;
        const text = getTextFromContent(data.message?.content || []);
        if (currentTurn) {
          currentTurn.userMessage = { eventId: event.id, text, ts: event.ts };
        } else {
          // Message outside a turn (shouldn't happen normally, but handle gracefully)
          const orphanTurn: TurnAccumulator = {
            turnId: `orphan-${event.id}`,
            startTs: event.ts,
            userMessage: { eventId: event.id, text, ts: event.ts },
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
        const data = event.data as ReasonCompletedData;
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
        const data = event.data as ActStartedData;
        currentIteration.toolGroup = {
          eventId: event.id,
          ts: event.ts,
          tools: data.tool_calls.map((tc) => {
            const result = toolResults.get(tc.id);
            return {
              id: tc.id,
              name: tc.name,
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
        const data = event.data as ActCompletedData;
        currentIteration.toolGroup.successCount = data.success_count;
        currentIteration.toolGroup.errorCount = data.error_count;
        currentIteration.toolGroup.durationMs = data.duration_ms;
        break;
      }

      case "output.message.completed": {
        if (!currentTurn) break;
        const data = event.data as OutputMessageCompletedData;
        const text = getTextFromContent(data.message?.content || []);
        const hasToolCalls = data.message?.content?.some((p) => p.type === "tool_call") ?? false;
        currentTurn.agentMessage = {
          eventId: event.id,
          text,
          ts: event.ts,
          hasToolCalls,
        };
        break;
      }

      case "turn.completed": {
        if (!currentTurn) break;
        const data = event.data as TurnCompletedData;
        currentTurn.completed = true;
        currentTurn.durationMs = data.duration_ms;
        currentTurn.iterationCount = data.iterations;
        currentTurn = null;
        currentIteration = null;
        break;
      }

      case "turn.failed": {
        if (!currentTurn) break;
        const data = event.data as TurnFailedData;
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

/**
 * Convert structured turns into React Flow nodes and edges.
 */
export function buildTrajectory(events: Event[]): { nodes: Node[]; edges: Edge[] } {
  const turns = buildTurns(events);
  const nodes: Node[] = [];
  const edges: Edge[] = [];
  let y = 0;
  let prevNodeId: string | null = null;

  function addNode(
    id: string,
    type: TrajectoryNodeType,
    data: TrajectoryNodeData,
    x: number = MAIN_X,
  ) {
    nodes.push({
      id,
      type,
      position: { x, y },
      data: data as unknown as Record<string, unknown>,
    });
  }

  function addEdge(source: string, target: string, animated = false) {
    edges.push({
      id: `e-${source}-${target}`,
      source,
      target,
      animated,
      style: { strokeWidth: 1.5 },
    });
  }

  function connectToPrev(nodeId: string, animated = false) {
    if (prevNodeId) {
      addEdge(prevNodeId, nodeId, animated);
    }
    prevNodeId = nodeId;
  }

  for (let turnIdx = 0; turnIdx < turns.length; turnIdx++) {
    const turn = turns[turnIdx];
    const turnPrefix = `turn-${turnIdx}`;

    // Turn start marker
    const turnStartId = `${turnPrefix}-start`;
    addNode(turnStartId, "turnStart", {
      label: `Turn ${turnIdx + 1}`,
      turnId: turn.turnId,
      iterations: turn.iterationCount ?? turn.iterations.length,
      durationMs: turn.durationMs,
      failed: turn.failed,
      errorMessage: turn.errorMessage,
      timestamp: turn.startTs,
    } as TurnNodeData);
    connectToPrev(turnStartId);
    y += SPACING_MARKER;

    // User message
    if (turn.userMessage) {
      const umId = `${turnPrefix}-user`;
      addNode(umId, "userMessage", {
        label: "User",
        preview: truncate(turn.userMessage.text, 120),
        eventId: turn.userMessage.eventId,
        timestamp: turn.userMessage.ts,
      } as UserMessageNodeData);
      connectToPrev(umId);
      y += SPACING_CONTENT;
    }

    // Iterations (reasoning + tool calls)
    for (let iterIdx = 0; iterIdx < turn.iterations.length; iterIdx++) {
      const iter = turn.iterations[iterIdx];

      // Reasoning node
      if (iter.reasoning) {
        const rId = `${turnPrefix}-reason-${iterIdx}`;
        addNode(rId, "reasoning", {
          label: `Reasoning ${iterIdx + 1}`,
          iteration: iterIdx + 1,
          hasToolCalls: iter.reasoning.hasToolCalls,
          toolCallCount: iter.reasoning.toolCallCount,
          durationMs: iter.reasoning.durationMs,
          eventId: iter.reasoning.eventId,
          timestamp: iter.reasoning.ts,
        } as ReasoningNodeData);
        connectToPrev(rId);
        y += SPACING_COMPACT;
      }

      // Tool group node
      if (iter.toolGroup && iter.toolGroup.tools.length > 0) {
        const tgId = `${turnPrefix}-tools-${iterIdx}`;
        addNode(tgId, "toolGroup", {
          label: `Tools (${iter.toolGroup.tools.length})`,
          tools: iter.toolGroup.tools,
          successCount: iter.toolGroup.successCount,
          errorCount: iter.toolGroup.errorCount,
          durationMs: iter.toolGroup.durationMs,
          eventId: iter.toolGroup.eventId,
          timestamp: iter.toolGroup.ts,
        } as ToolGroupNodeData);
        connectToPrev(tgId);
        // More tools = taller node
        y += SPACING_CONTENT + Math.max(0, (iter.toolGroup.tools.length - 3) * 20);
      }
    }

    // Agent message
    if (turn.agentMessage) {
      const amId = `${turnPrefix}-agent`;
      addNode(amId, "agentMessage", {
        label: "Agent",
        preview: truncate(turn.agentMessage.text, 120),
        eventId: turn.agentMessage.eventId,
        timestamp: turn.agentMessage.ts,
        hasToolCalls: turn.agentMessage.hasToolCalls,
      } as AgentMessageNodeData);
      connectToPrev(amId);
      y += SPACING_CONTENT;
    }

    // Turn end marker
    if (turn.completed) {
      const turnEndId = `${turnPrefix}-end`;
      addNode(turnEndId, "turnEnd", {
        label: turn.failed ? "Turn Failed" : "Turn Done",
        turnId: turn.turnId,
        iterations: turn.iterationCount ?? turn.iterations.length,
        durationMs: turn.durationMs,
        failed: turn.failed,
        errorMessage: turn.errorMessage,
        timestamp: turn.startTs,
      } as TurnNodeData);
      connectToPrev(turnEndId);
      y += SPACING_MARKER;
    }
  }

  return { nodes, edges };
}
