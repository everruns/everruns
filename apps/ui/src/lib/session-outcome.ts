import type { Session, SessionStatus } from "@/lib/api/types";

/** Execution in progress takes precedence over the last terminal outcome. */
export function sessionOutcome(
  session: Pick<Session, "status" | "activity" | "last_turn_status">,
  effectiveStatus: SessionStatus = session.status,
): {
  label: string;
  variant: "default" | "secondary" | "outline" | "success";
  isRunning: boolean;
  className?: string;
} {
  if (effectiveStatus === "active")
    return { label: "Running", variant: "default", isRunning: true };
  if (effectiveStatus === "waiting_for_tool_results")
    return { label: "Waiting on tools", variant: "outline", isRunning: true };
  if (effectiveStatus === "paused")
    return { label: "Paused", variant: "outline", isRunning: false };
  if (session.last_turn_status === "cancelled")
    return { label: "Stopped", variant: "secondary", isRunning: false };
  const outcome = session.last_turn_status ?? session.activity;
  if (outcome === "failed")
    return {
      label: "Last turn failed",
      variant: "outline",
      isRunning: false,
      className: "border-destructive/30 bg-destructive/5 text-destructive",
    };
  if (outcome === "completed") return { label: "Completed", variant: "success", isRunning: false };
  return {
    label: effectiveStatus === "started" ? "New" : "Idle",
    variant: "secondary",
    isRunning: false,
  };
}
