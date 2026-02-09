// Main trajectory visualization component.
// Uses React Flow with MiniMap for navigating large agent trajectories.
// Supports thousands of nodes via viewport culling.

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

import type { Event } from "@/lib/api/types";
import { buildTrajectory } from "./trajectory-utils";
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
    case "turnStart":
    case "turnEnd":
      return "#94a3b8"; // slate
    default:
      return "#6b7280";
  }
}

export function TrajectoryView({ events, colorMode }: TrajectoryViewProps) {
  // Build nodes/edges from events - recomputed when events change (SSE updates)
  const { nodes, edges } = useMemo(() => buildTrajectory(events), [events]);

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
    <div className="w-full h-full">
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
          type: "smoothstep",
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
  );
}
