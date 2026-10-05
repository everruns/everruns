// Shared vocabulary for the Sandbox fleet: state and attention labels, colors
// and small formatters. State is always readable as text; color is a second cue.

import type { SandboxAttention, SandboxFleetState } from "@/lib/api/sandboxes";
import { cn } from "@/lib/utils";

export const SANDBOX_STATE_LABELS: Record<SandboxFleetState, string> = {
  running: "Running",
  paused: "Paused",
  lost: "Lost",
  starting: "Starting",
  failed: "Failed",
  not_started: "Not started",
  deleted: "Deleted",
};

/** Order of the state chips, live states first. */
export const SANDBOX_STATE_ORDER: SandboxFleetState[] = [
  "running",
  "paused",
  "lost",
  "starting",
  "failed",
  "not_started",
  "deleted",
];

export const LIVE_STATES: SandboxFleetState[] = ["running", "paused", "lost", "starting", "failed"];

/** Background class of a state's dot or timeline bar. */
export function sandboxStateTone(state: string): string {
  switch (state) {
    case "running":
      return "bg-success";
    case "paused":
      return "bg-warning";
    case "lost":
    case "failed":
      return "bg-destructive";
    case "starting":
      return "bg-accent";
    default:
      return "bg-muted-foreground/50";
  }
}

export function sandboxStateLabel(state: string): string {
  return SANDBOX_STATE_LABELS[state as SandboxFleetState] ?? state.replaceAll("_", " ");
}

export function SandboxStateBadge({ state, className }: { state: string; className?: string }) {
  return (
    <span
      className={cn("inline-flex shrink-0 items-center gap-1.5 text-xs font-medium", className)}
    >
      <span aria-hidden="true" className={cn("size-1.5 rounded-full", sandboxStateTone(state))} />
      {sandboxStateLabel(state)}
    </span>
  );
}

export const SANDBOX_ATTENTION_COPY: Record<SandboxAttention, { title: string; hint: string }> = {
  lost: {
    title: "Lost, not rebuilt yet",
    hint: "The provider resource is gone. The next tool call rebuilds it from the last checkpoint.",
  },
  failed: {
    title: "Failed to start",
    hint: "The provider could not create the Sandbox.",
  },
  init_failed: {
    title: "Init commands failed",
    hint: "The template's setup commands exited with an error.",
  },
  idle_running: {
    title: "Running with no activity for over an hour",
    hint: "It keeps costing compute. Pause it, or check the template's idle pause setting.",
  },
  cleanup_failed: {
    title: "Provider cleanup failed",
    hint: "Everruns could not release the provider resource and keeps retrying. Check the provider connection.",
  },
};

export function attentionTitle(reason: string): string {
  return SANDBOX_ATTENTION_COPY[reason as SandboxAttention]?.title ?? reason.replaceAll("_", " ");
}

const PROVIDER_LABELS: Record<string, string> = {
  daytona: "Daytona",
  modal: "Modal",
  docker: "Docker",
  e2b: "E2B",
  sprites: "Sprites",
  deno: "Deno",
  bashkit: "Bashkit",
  host: "Host",
};

export function providerLabel(provider: string): string {
  return PROVIDER_LABELS[provider] ?? provider.charAt(0).toUpperCase() + provider.slice(1);
}

/** Hours with one decimal under 100, whole hours above. */
export function formatRunningTime(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.round(seconds / 60)}m`;
  const hours = seconds / 3600;
  return `${hours < 100 ? hours.toFixed(1) : Math.round(hours)} h`;
}

/** Age of a Sandbox, short form for table columns. */
export function formatAge(from: string, to: string | null = null): string {
  const end = to ? new Date(to).getTime() : Date.now();
  const seconds = Math.max(0, Math.round((end - new Date(from).getTime()) / 1000));
  if (seconds < 3600) return `${Math.max(1, Math.round(seconds / 60))}m`;
  if (seconds < 86_400) return `${Math.round(seconds / 3600)}h`;
  return `${Math.round(seconds / 86_400)}d`;
}
