"use client";

import { useId } from "react";
import type { MonitoringSnapshot } from "@/lib/api/types";
import { number, perSecond, phaseCount, rate, phases } from "./model";

export function ExecutionFlow({
  data,
  selected,
  onSelect,
}: {
  data: MonitoringSnapshot;
  selected: string | null;
  onSelect: (phase: string) => void;
}) {
  const id = useId().replaceAll(":", "");
  const nodes = [
    { phase: "runnable", x: 25, y: 108, w: 160, h: 135 },
    { phase: "model", x: 345, y: 42, w: 190, h: 140 },
    { phase: "tools", x: 345, y: 302, w: 190, h: 140 },
    { phase: "waiting", x: 765, y: 302, w: 190, h: 140 },
    { phase: "setup", x: 25, y: 327, w: 160, h: 105 },
  ];
  const completed = rate(data, "turn.completed");
  const failed = rate(data, "turn.failed") + rate(data, "turn.sealed");
  const edges = [
    {
      d: "M185 159 C245 159 282 112 329 112",
      color: "hsl(var(--accent))",
      event: "turn.started",
      label: "Turn start",
      x: 241,
      y: 120,
    },
    {
      d: "M440 182 L440 287",
      color: "hsl(var(--success))",
      event: "act.started",
      label: "Tool batches",
      x: 510,
      y: 237,
    },
    {
      d: "M345 369 C210 369 213 127 329 127",
      color: "hsl(var(--info))",
      event: null,
      label: "Next iteration",
      x: 250,
      y: 270,
    },
    {
      d: "M535 112 L747 112",
      color: "hsl(var(--muted-foreground))",
      event: "turn.completed",
      label: "Turn finished",
      x: 642,
      y: 75,
    },
    {
      d: "M535 370 L747 370",
      color: "hsl(var(--muted-foreground))",
      event: null,
      label: "Paused",
      x: 642,
      y: 335,
    },
    {
      d: "M765 320 C656 320 656 155 552 155",
      color: "hsl(var(--muted-foreground))",
      event: null,
      label: "Resolved",
      x: 654,
      y: 257,
    },
  ];
  return (
    <section className="border bg-card p-4" data-testid="monitoring-flow">
      <div className="flex items-baseline justify-between gap-4">
        <h2 className="text-sm font-semibold">
          {data.scope === "fleet" ? "Fleet execution" : "Organization execution"}
        </h2>
        <span className="text-xs text-muted-foreground">
          Rates over the last {data.window_seconds}s
        </span>
      </div>
      <div className="overflow-x-auto">
        <svg
          viewBox="0 0 980 485"
          className="w-full min-w-[600px]"
          role="group"
          aria-label="Execution flow: runnable work, model iterations, tools, waiting, and completion"
        >
          <defs>
            <pattern id={`${id}-waiting`} width="7" height="7" patternUnits="userSpaceOnUse">
              <path
                d="M-2 2L2-2M0 7L7 0M5 9L9 5"
                stroke="hsl(var(--muted-foreground))"
                strokeOpacity=".16"
              />
            </pattern>
            {edges.map((edge, index) => (
              <marker
                key={index}
                id={`${id}-arrow-${index}`}
                markerWidth="10"
                markerHeight="10"
                refX="7"
                refY="3"
                orient="auto"
                markerUnits="userSpaceOnUse"
              >
                <path d="M0 0L8 3L0 6Z" fill={edge.color} />
              </marker>
            ))}
          </defs>
          {edges.map((edge, index) => (
            <g key={edge.label}>
              <path
                d={edge.d}
                fill="none"
                stroke={edge.color}
                strokeOpacity=".38"
                strokeWidth={
                  edge.event ? 5 + Math.min(14, Math.sqrt(rate(data, edge.event)) * 3) : 5
                }
                markerEnd={`url(#${id}-arrow-${index})`}
                strokeDasharray={edge.event ? undefined : "5 4"}
              />
              <text
                x={edge.x}
                y={edge.y}
                textAnchor="middle"
                fontSize="12"
                fill="hsl(var(--foreground))"
              >
                {edge.label}
              </text>
              {edge.event && edge.label !== "Next iteration" && (
                <text
                  x={edge.x}
                  y={edge.y + 19}
                  textAnchor="middle"
                  fontSize="11"
                  fill="hsl(var(--muted-foreground))"
                >
                  {perSecond(rate(data, edge.event))}/s
                </text>
              )}
            </g>
          ))}
          {nodes.map((node) => {
            const phase = phases.find((p) => p.key === node.phase)!;
            return (
              <g
                key={node.phase}
                role="button"
                tabIndex={0}
                aria-label={`${phase.label}: ${phaseCount(data, node.phase)} sessions`}
                aria-pressed={selected === node.phase}
                onClick={() => onSelect(node.phase)}
                onKeyDown={(event) => {
                  if (event.key === "Enter" || event.key === " ") {
                    event.preventDefault();
                    onSelect(node.phase);
                  }
                }}
                className="cursor-pointer focus:outline-2 focus:outline-ring"
              >
                <rect
                  x={node.x}
                  y={node.y}
                  width={node.w}
                  height={node.h}
                  fill={phase.color}
                  fillOpacity=".08"
                  stroke={selected === node.phase ? "hsl(var(--accent))" : phase.color}
                  strokeOpacity=".65"
                  strokeWidth={selected === node.phase ? 2 : 1}
                />
                {node.phase === "waiting" && (
                  <rect
                    x={node.x}
                    y={node.y}
                    width={node.w}
                    height={node.h}
                    fill={`url(#${id}-waiting)`}
                  />
                )}
                <text
                  x={node.x + node.w / 2}
                  y={node.y + 36}
                  textAnchor="middle"
                  fontSize="14"
                  fontWeight="600"
                  fill="hsl(var(--foreground))"
                >
                  {phase.label}
                </text>
                <text
                  x={node.x + node.w / 2}
                  y={node.y + 70}
                  textAnchor="middle"
                  fontSize="27"
                  fontWeight="600"
                  fill="hsl(var(--foreground))"
                >
                  {number(phaseCount(data, node.phase))}
                </text>
                <text
                  x={node.x + node.w / 2}
                  y={node.y + 91}
                  textAnchor="middle"
                  fontSize="11"
                  fill="hsl(var(--muted-foreground))"
                >
                  current sessions
                </text>
              </g>
            );
          })}
          <g>
            <rect
              x="765"
              y="42"
              width="190"
              height="140"
              fill="hsl(var(--muted))"
              stroke="hsl(var(--border))"
            />
            <text
              x="860"
              y="85"
              textAnchor="middle"
              fontSize="14"
              fontWeight="600"
              fill="hsl(var(--foreground))"
            >
              Completed
            </text>
            <text
              x="860"
              y="122"
              textAnchor="middle"
              fontSize="27"
              fontWeight="600"
              fill="hsl(var(--foreground))"
            >
              {perSecond(completed)}/s
            </text>
            <text
              x="860"
              y="147"
              textAnchor="middle"
              fontSize="11"
              fill="hsl(var(--muted-foreground))"
            >
              turns finished
            </text>
          </g>
          <g>
            <rect
              x="765"
              y="210"
              width="190"
              height="62"
              fill="hsl(var(--destructive) / .05)"
              stroke="hsl(var(--destructive) / .35)"
            />
            <text x="860" y="237" textAnchor="middle" fontSize="12" fill="hsl(var(--foreground))">
              Failed / sealed
            </text>
            <text x="860" y="257" textAnchor="middle" fontSize="13" fill="hsl(var(--destructive))">
              {perSecond(failed)}/s
            </text>
          </g>
        </svg>
      </div>
      <div className="flex flex-wrap gap-x-6 gap-y-2 border-t pt-3 text-xs text-muted-foreground">
        <span>{number(data.events["reason.started"] ?? 0)} model iterations</span>
        <span>{number(data.events["tool.started"] ?? 0)} tool calls started</span>
        <span>{number(data.events["tool.completed"] ?? 0)} tool calls settled</span>
        <span>Counts over the last {data.window_seconds}s</span>
      </div>
      <p className="mt-3 text-xs text-muted-foreground">
        Nodes show current sessions. Measured link widths show event rates; dashed paths show the
        lifecycle. Iterations continue within the same turn.
      </p>
    </section>
  );
}
