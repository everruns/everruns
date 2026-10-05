"use client";

// The fleet's summary strip. Every figure that names a set of Sandboxes is also
// the button that filters the table to that set.

import type { SandboxFleetStats } from "@/lib/api/sandboxes";
import { cn } from "@/lib/utils";
import {
  LIVE_STATES,
  SANDBOX_STATE_ORDER,
  formatRunningTime,
  providerLabel,
  sandboxStateLabel,
  sandboxStateTone,
} from "./sandbox-display";

function countOf(stats: SandboxFleetStats | undefined, state: string): number {
  return stats?.by_state.find((entry) => entry.key === state)?.count ?? 0;
}

function Tile({
  label,
  value,
  hint,
  active,
  onClick,
  tone,
  children,
}: {
  label: string;
  value: string;
  hint?: React.ReactNode;
  active?: boolean;
  onClick?: () => void;
  tone?: "attention";
  children?: React.ReactNode;
}) {
  const body = (
    <>
      <span className="font-mono text-[11px] uppercase tracking-[0.08em] text-muted-foreground">
        {label}
      </span>
      <span
        className={cn(
          "text-2xl font-semibold tabular-nums tracking-tight text-foreground",
          tone === "attention" && value !== "0" && "text-destructive",
        )}
      >
        {value}
      </span>
      {children}
      {hint ? (
        <span className="text-[13px] text-muted-foreground">{hint}</span>
      ) : null}
    </>
  );
  const className = cn(
    "flex min-w-0 flex-col gap-1.5 border bg-card p-4 text-left",
    onClick &&
      "transition-colors hover:bg-muted/50 focus-visible:outline-2 focus-visible:outline-ring",
    active && "border-primary",
  );
  return onClick ? (
    <button
      type="button"
      className={className}
      onClick={onClick}
      aria-pressed={active}
    >
      {body}
    </button>
  ) : (
    <div className={className}>{body}</div>
  );
}

export function SandboxFleetSummary({
  stats,
  state,
  needsAttention,
  onState,
  onNeedsAttention,
}: {
  stats: SandboxFleetStats | undefined;
  state: string;
  needsAttention: boolean;
  onState: (state: string) => void;
  onNeedsAttention: (value: boolean) => void;
}) {
  const live = LIVE_STATES.reduce((sum, s) => sum + countOf(stats, s), 0);
  const running = countOf(stats, "running");
  const paused = countOf(stats, "paused");
  const window = stats?.window_days ?? 7;
  const created = stats?.created_in_window ?? 0;
  const prior = stats?.created_in_prior_window ?? 0;
  const trend =
    prior === 0
      ? created > 0
        ? "none the week before"
        : "none the week before either"
      : `${created >= prior ? "up" : "down"} ${Math.round((Math.abs(created - prior) / prior) * 100)}% on the week before`;
  const providers = stats?.live_by_provider ?? [];

  return (
    <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-5">
      <Tile
        label="Live now"
        value={String(live)}
        active={state === "live" && !needsAttention}
        onClick={() => {
          onNeedsAttention(false);
          onState("live");
        }}
        hint={`${running} running, ${paused} paused`}
      >
        {live > 0 ? (
          <span
            className="flex h-1.5 w-full overflow-hidden bg-muted"
            aria-hidden="true"
          >
            {LIVE_STATES.map((s) => {
              const count = countOf(stats, s);
              return count > 0 ? (
                <span
                  key={s}
                  className={sandboxStateTone(s)}
                  style={{ width: `${(count / live) * 100}%` }}
                />
              ) : null;
            })}
          </span>
        ) : null}
      </Tile>
      <Tile
        label={`Created, ${window} days`}
        value={String(created)}
        hint={trend}
      />
      <Tile
        label={`Running time, ${window} days`}
        value={formatRunningTime(stats?.running_seconds_in_window ?? 0)}
        hint={`${stats?.recoveries_in_window ?? 0} lost and rebuilt`}
      />
      <Tile
        label="Live by provider"
        value={String(providers.length)}
        hint={
          providers.length === 0
            ? "Nothing live"
            : providers
                .map((p) => `${providerLabel(p.key)} ${p.count}`)
                .join(", ")
        }
      />
      <Tile
        label="Needs attention"
        value={String(stats?.needs_attention ?? 0)}
        tone="attention"
        active={needsAttention}
        onClick={() => {
          onNeedsAttention(!needsAttention);
          onState("all");
        }}
        hint="Lost, failed, idle or stuck in cleanup"
      />
      <div
        className="flex flex-wrap gap-1.5 sm:col-span-2 xl:col-span-5"
        role="group"
        aria-label="Filter by state"
      >
        <StateChip
          label="All"
          count={stats?.by_state.reduce((sum, entry) => sum + entry.count, 0)}
          active={state === "all" && !needsAttention}
          onClick={() => {
            onNeedsAttention(false);
            onState("all");
          }}
        />
        {SANDBOX_STATE_ORDER.map((s) => (
          <StateChip
            key={s}
            label={sandboxStateLabel(s)}
            tone={sandboxStateTone(s)}
            count={countOf(stats, s)}
            active={state === s && !needsAttention}
            onClick={() => {
              onNeedsAttention(false);
              onState(s);
            }}
          />
        ))}
      </div>
    </div>
  );
}

function StateChip({
  label,
  count,
  tone,
  active,
  onClick,
}: {
  label: string;
  count?: number;
  tone?: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      className={cn(
        "inline-flex items-center gap-1.5 border px-2.5 py-1 text-xs transition-colors",
        active
          ? "border-primary bg-background font-medium text-foreground"
          : "border-transparent bg-muted text-muted-foreground hover:text-foreground",
      )}
    >
      {tone ? (
        <span
          aria-hidden="true"
          className={cn("size-1.5 rounded-full", tone)}
        />
      ) : null}
      {label}
      {count !== undefined ? (
        <span className="tabular-nums">{count}</span>
      ) : null}
    </button>
  );
}
