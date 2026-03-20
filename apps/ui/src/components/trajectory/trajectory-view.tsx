// Main trajectory visualization component.
// Uses React Flow with MiniMap for navigating large agent trajectories.
//
// Performance for 1000+ turns:
// - Memoization gated on structural event count (skips SSE deltas)
// - React Flow viewport culling (only visible nodes in DOM)
// - Nodes are not draggable/connectable/selectable (reduces event overhead)
// - MiniMap for efficient navigation of large flows

"use client";

import { useMemo, useCallback } from "react";
import {
  ReactFlow,
  MiniMap,
  Controls,
  Background,
  BackgroundVariant,
  type ColorMode,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { Repeat, Wrench, XCircle, Clock } from "lucide-react";

import type { Event } from "@/lib/api/types";
import { formatDurationCompact } from "@/lib/formatting";
import { buildTrajectory, countStructuralEvents, type TrajectoryStats } from "./trajectory-utils";
import { trajectoryNodeTypes } from "./trajectory-nodes";

interface TrajectoryViewProps {
  events: Event[];
  colorMode?: ColorMode;
}

// Minimap node color based on node type
function miniMapNodeColor(node: { type?: string }): string {
  switch (node.type) {
    case "userMessage":
      return "#3b82f6"; // blue
    case "agentMessage":
      return "#8b5cf6"; // violet
    case "reasoning":
      return "#f59e0b"; // amber
    case "toolGroup":
      return "#10b981"; // emerald
    case "turnGroup":
      return "#94a3b8"; // slate
    default:
      return "#6b7280";
  }
}

function formatDuration(ms: number): string {
  if (ms < 60_000) return formatDurationCompact(ms);
  const mins = Math.floor(ms / 60_000);
  const secs = ((ms % 60_000) / 1000).toFixed(0);
  return `${mins}m ${secs}s`;
}

function StatsBar({ stats }: { stats: TrajectoryStats }) {
  return (
    <div className="flex items-center gap-4 px-4 py-2 border-b text-xs text-muted-foreground bg-muted/30">
      <span className="flex items-center gap-1">
        <Repeat className="w-3 h-3" />
        {stats.turnCount} turn{stats.turnCount !== 1 ? "s" : ""}
        {stats.totalIterations > 0 && (
          <span className="text-muted-foreground/60">
            ({stats.totalIterations} iter{stats.totalIterations !== 1 ? "s" : ""})
          </span>
        )}
      </span>
      {stats.totalToolCalls > 0 && (
        <span className="flex items-center gap-1">
          <Wrench className="w-3 h-3" />
          {stats.totalToolCalls} tool call{stats.totalToolCalls !== 1 ? "s" : ""}
        </span>
      )}
      {stats.totalToolErrors > 0 && (
        <span className="flex items-center gap-1 text-red-500">
          <XCircle className="w-3 h-3" />
          {stats.totalToolErrors} error{stats.totalToolErrors !== 1 ? "s" : ""}
        </span>
      )}
      {stats.failedTurns > 0 && (
        <span className="flex items-center gap-1 text-red-500">
          {stats.failedTurns} failed turn{stats.failedTurns !== 1 ? "s" : ""}
        </span>
      )}
      {stats.totalDurationMs > 0 && (
        <span className="flex items-center gap-1">
          <Clock className="w-3 h-3" />
          {formatDuration(stats.totalDurationMs)}
        </span>
      )}
    </div>
  );
}

export function TrajectoryView({ events, colorMode }: TrajectoryViewProps) {
  // Count structural events as a stable primitive memoization key.
  // This number only changes when a turn/reason/act/tool/message event arrives,
  // NOT on every output.message.delta or reason.thinking.delta (which fire 10-30x/sec).
  const structuralCount = useMemo(() => countStructuralEvents(events), [events]);

  // Build nodes/edges/stats only when structural events change
  const { nodes, edges, stats } = useMemo(
    () => buildTrajectory(events),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- intentional: gate on structuralCount, not events ref
    [structuralCount],
  );

  // Fit view on mount
  const onInit = useCallback((instance: { fitView: (opts?: { padding?: number }) => void }) => {
    requestAnimationFrame(() => {
      instance.fitView({ padding: 0.2 });
    });
  }, []);

  if (nodes.length === 0) {
    return (
      <div className="flex items-center justify-center h-full text-muted-foreground">
        <div className="text-center">
          <p className="text-lg font-medium">No trajectory data yet</p>
          <p className="text-sm">Trajectory will appear as the agent processes turns</p>
        </div>
      </div>
    );
  }

  return (
    <div className="w-full h-full flex flex-col">
      <StatsBar stats={stats} />
      <div className="flex-1">
        <ReactFlow
          nodes={nodes}
          edges={edges}
          nodeTypes={trajectoryNodeTypes}
          onInit={onInit}
          colorMode={colorMode ?? "system"}
          fitView
          minZoom={0.01}
          maxZoom={2}
          nodesDraggable={false}
          nodesConnectable={false}
          elementsSelectable={false}
          panOnScroll
          zoomOnScroll
          defaultEdgeOptions={{
            type: "bezier",
            animated: false,
            style: { strokeWidth: 1.5 },
          }}
        >
          <Controls showInteractive={false} />
          <MiniMap
            nodeColor={miniMapNodeColor}
            nodeStrokeWidth={0}
            maskColor="rgba(0, 0, 0, 0.15)"
            className="!bg-background !border-border"
            pannable
            zoomable
          />
          <Background variant={BackgroundVariant.Dots} gap={20} size={1} />
        </ReactFlow>
      </div>
    </div>
  );
}
