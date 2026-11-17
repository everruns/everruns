// How the `tool_approval` gate picks a session's mode, mirrored for display.
// The gate (resolve_mode in crates/core/src/builtins/tool_approval.rs) is the
// authority; the UI only uses this to label what a session runs in.
import { APPROVAL_MODES, type ApprovalModeName } from "@/lib/api/types";

/** `mode`, or the next more careful mode the organization allows. */
export function capApprovalMode(
  mode: ApprovalModeName,
  allowed: readonly string[],
): ApprovalModeName {
  if (allowed.length === 0) return mode;
  const start = APPROVAL_MODES.indexOf(mode);
  return (
    APPROVAL_MODES.slice(start).find((candidate) => allowed.includes(candidate)) ?? "protective"
  );
}

/**
 * The mode a session runs in: the person's switch, else the agent's mode.
 * A switch to Off runs as Auto when the agent's manager turned approvals on.
 * The owner's personal default sits between the two on the server; a viewer
 * cannot read another person's preference, so the label leaves it out.
 */
export function effectiveApprovalMode(
  switched: ApprovalModeName | null,
  agentMode: ApprovalModeName,
  allowed: readonly string[],
): ApprovalModeName {
  let mode = switched ?? agentMode;
  if (mode === "off" && agentMode !== "off") mode = "auto";
  return capApprovalMode(mode, allowed);
}

/** Modes a person may switch a session to, in the order the menu lists them. */
export function switchableModes(allowed: readonly string[]): ApprovalModeName[] {
  const order: ApprovalModeName[] = ["auto", "normal", "protective", "off"];
  return order.filter((mode) => allowed.length === 0 || allowed.includes(mode));
}
