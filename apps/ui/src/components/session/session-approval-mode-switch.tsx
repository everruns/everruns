"use client";

// Session header → approval mode switch. Writes the session's reserved
// `tool_approval/mode` key through PUT /v1/sessions/{id}/approval-mode; the
// gate reads it on every call, so a change applies from the next tool call.
// Modes the organization does not allow are hidden. Off stays listed but
// disabled when the agent's manager turned approvals on: the gate would run it
// as Auto, and a disabled row says so instead of silently doing something else.

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronDown, ShieldCheck } from "lucide-react";
import { useUiTranslations } from "@/providers/locale-provider";
import { buttonVariants } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuPositioner,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { MODE_LABELS, isApprovalMode } from "@/components/tool-approval/approval-controls";
import {
  getSessionApprovalMode,
  setSessionApprovalMode,
  type Agent,
  type ApprovalModeName,
} from "@/lib/api/types";
import { queryKeys } from "@/lib/query-keys";
import { effectiveApprovalMode, switchableModes } from "@/lib/tool-approval";
import { cn } from "@/lib/utils";
import { useOrg } from "@/providers/org-provider";

const DEFAULT_VALUE = "default";

const SWITCH_HINTS: Record<ApprovalModeName, string> = {
  auto: "A reviewer answers routine calls; you get the unclear ones",
  normal: "Asks before risky calls",
  protective: "Asks before any change",
  off: "Never asks",
};

/** The agent's own approval mode, or null when it has no approval gate. */
export function agentApprovalMode(agent: Agent | undefined): ApprovalModeName | null {
  const capability = agent?.capabilities?.find((entry) => entry.ref === "tool_approval");
  if (!capability) return null;
  const mode = capability.config?.mode;
  return isApprovalMode(mode) ? mode : "normal";
}

export function SessionApprovalModeSwitch({
  sessionId,
  agentMode,
}: {
  sessionId: string;
  agentMode: ApprovalModeName;
}) {
  const ui = useUiTranslations();
  const { currentOrg } = useOrg();
  const queryClient = useQueryClient();
  const queryKey = queryKeys.sessions.approvalMode(currentOrg?.public_id, sessionId);
  const { data } = useQuery({
    queryKey,
    queryFn: () => getSessionApprovalMode(sessionId),
    staleTime: 30_000,
  });
  const change = useMutation({
    mutationFn: (mode: ApprovalModeName | null) => setSessionApprovalMode(sessionId, mode),
    onSuccess: (next) => queryClient.setQueryData(queryKey, next),
  });

  if (!data) return null;
  const switched = isApprovalMode(data.mode) ? data.mode : null;
  const allowed = data.allowed_modes;
  const effective = effectiveApprovalMode(switched, agentMode, allowed);
  const modes = switchableModes(allowed);
  const defaultMode = effectiveApprovalMode(null, agentMode, allowed);

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        className={cn(buttonVariants({ variant: "outline", size: "sm" }), "gap-1")}
        aria-label={ui("Approval mode: {mode}", { mode: ui(MODE_LABELS[effective]) })}
        data-testid="session-approval-mode"
      >
        <ShieldCheck className="icon-sharp h-4 w-4" />
        {ui("Approvals: {mode}", { mode: ui(MODE_LABELS[effective]) })}
        <ChevronDown className="h-3.5 w-3.5 opacity-60" />
      </DropdownMenuTrigger>
      <DropdownMenuPositioner align="end">
        <DropdownMenuContent className="w-80">
          <DropdownMenuRadioGroup
            value={switched ?? DEFAULT_VALUE}
            onValueChange={(value) => {
              if (change.isPending) return;
              change.mutate(value === DEFAULT_VALUE ? null : (value as ApprovalModeName));
            }}
          >
            {modes.map((mode) => {
              const blocked = mode === "off" && agentMode !== "off";
              return (
                <DropdownMenuRadioItem key={mode} value={mode} disabled={blocked}>
                  <span className="min-w-0">
                    <span className="block font-medium">
                      {mode === agentMode
                        ? ui("{mode} (agent default)", { mode: ui(MODE_LABELS[mode]) })
                        : ui(MODE_LABELS[mode])}
                    </span>
                    <span className="block text-xs text-muted-foreground">
                      {blocked
                        ? ui("Not available: this agent's manager turned approvals on")
                        : ui(SWITCH_HINTS[mode])}
                    </span>
                  </span>
                </DropdownMenuRadioItem>
              );
            })}
            <DropdownMenuSeparator />
            <DropdownMenuRadioItem value={DEFAULT_VALUE}>
              <span className="min-w-0">
                <span className="block font-medium">{ui("Use default")}</span>
                <span className="block text-xs text-muted-foreground">
                  {ui("Your own default, else the agent's mode ({mode})", {
                    mode: ui(MODE_LABELS[defaultMode]),
                  })}
                </span>
              </span>
            </DropdownMenuRadioItem>
          </DropdownMenuRadioGroup>
          <p className="px-2 pt-1.5 pb-1 text-xs text-muted-foreground">
            {change.isError
              ? ui("Could not change the mode. You may not be allowed to manage this session.")
              : ui("Applies from the next tool call.")}
          </p>
        </DropdownMenuContent>
      </DropdownMenuPositioner>
    </DropdownMenu>
  );
}
