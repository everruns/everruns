/**
 * "In this session" rail (EVE-759, area F) — lists the active participants of a
 * multi-party session with role/kind badges, an Invite-agent control, and a
 * per-member Leave control.
 *
 * Guarded: an ordinary 1:1 session (host only, nobody has left) renders nothing,
 * so single-host sessions look exactly as before. Hidden below `lg` like the
 * ChatNavRail so narrow viewports keep the full transcript width.
 */
"use client";

import { useMemo, useState } from "react";
import { Bot, Loader2, LogOut, User, UserPlus } from "lucide-react";
import type { SessionParticipant } from "@/lib/api/types";
import {
  useAddSessionParticipant,
  useAgents,
  useLeaveSessionParticipant,
  useSessionParticipants,
} from "@/hooks";
import { getDisplayName } from "@/lib/entity-lifecycle";
import { getSessionParticipantLabel } from "@/lib/session-participant-label";
import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { cn } from "@/lib/utils";

interface SessionParticipantsRailProps {
  sessionId: string;
  className?: string;
}

export function SessionParticipantsRail({ sessionId, className }: SessionParticipantsRailProps) {
  const { data: participants } = useSessionParticipants(sessionId);
  const { data: agents } = useAgents();
  const addParticipant = useAddSessionParticipant(sessionId);
  const leaveParticipant = useLeaveSessionParticipant(sessionId);
  const [inviteOpen, setInviteOpen] = useState(false);

  const agentNameById = useMemo(() => {
    const map = new Map<string, string>();
    for (const agent of agents ?? []) {
      map.set(agent.id, getDisplayName(agent));
    }
    return map;
  }, [agents]);

  const active = useMemo(() => (participants ?? []).filter((p) => !p.left_at), [participants]);
  const left = useMemo(() => (participants ?? []).filter((p) => p.left_at), [participants]);

  const activeAgentIds = useMemo(() => {
    const ids = new Set<string>();
    for (const p of active) {
      if (p.kind === "agent" && p.agent_id) ids.add(p.agent_id);
    }
    return ids;
  }, [active]);

  const invitableAgents = useMemo(
    () => (agents ?? []).filter((agent) => !activeAgentIds.has(agent.id)),
    [agents, activeAgentIds],
  );

  // Guard: keep ordinary 1:1 sessions (host only) untouched.
  if (active.length < 2 && left.length === 0) return null;

  const participantLabel = (p: SessionParticipant): string =>
    getSessionParticipantLabel(p, agentNameById);

  const handleInvite = async (agentId: string) => {
    try {
      await addParticipant.mutateAsync({ kind: "agent", agent_id: agentId, role: "member" });
      setInviteOpen(false);
    } catch (error) {
      console.error("Failed to invite agent:", error);
    }
  };

  const renderRow = (p: SessionParticipant, isLeft: boolean) => {
    const isHost = p.role === "host";
    const isAgent = p.kind === "agent";
    return (
      <li
        key={p.id}
        className={cn(
          "flex items-start gap-2 border border-border/60 bg-card/60 px-2 py-2",
          isLeft && "opacity-50",
        )}
      >
        <Avatar className="size-6 rounded-none">
          <AvatarFallback className="rounded-none bg-muted text-muted-foreground">
            {isAgent ? <Bot className="h-3.5 w-3.5" /> : <User className="h-3.5 w-3.5" />}
          </AvatarFallback>
        </Avatar>
        <div className="min-w-0 flex-1">
          <div className="truncate text-xs font-medium text-foreground">{participantLabel(p)}</div>
          <div className="mt-1 flex flex-wrap items-center gap-1">
            <Badge variant={isHost ? "accent" : "outline"}>{isHost ? "Host" : "Member"}</Badge>
            <Badge variant="secondary">{isAgent ? "Agent" : "User"}</Badge>
          </div>
        </div>
        {!isHost && !isLeft && (
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            title="Remove from session"
            aria-label={`Remove ${participantLabel(p)} from session`}
            disabled={leaveParticipant.isPending}
            onClick={() => leaveParticipant.mutate(p.id)}
          >
            <LogOut className="h-3.5 w-3.5" />
          </Button>
        )}
      </li>
    );
  };

  return (
    <aside
      className={cn(
        "hidden w-64 shrink-0 flex-col border-l border-border bg-background lg:flex",
        className,
      )}
      aria-label="Session participants"
    >
      <div className="flex items-center justify-between border-b border-border px-3 py-2.5">
        <span className="text-[11px] font-medium uppercase tracking-[0.18em] text-muted-foreground">
          In this session
        </span>
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          title="Invite agent"
          aria-label="Invite agent"
          onClick={() => setInviteOpen(true)}
        >
          <UserPlus className="h-3.5 w-3.5" />
        </Button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-3 py-3">
        <ul className="space-y-2">{active.map((p) => renderRow(p, false))}</ul>
        {left.length > 0 && (
          <>
            <div className="mt-4 mb-2 text-[10px] font-medium uppercase tracking-[0.18em] text-muted-foreground">
              Left
            </div>
            <ul className="space-y-2">{left.map((p) => renderRow(p, true))}</ul>
          </>
        )}
      </div>

      <Dialog open={inviteOpen} onOpenChange={setInviteOpen}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Invite agent</DialogTitle>
            <DialogDescription>
              Add an agent as a member of this session. Members can be addressed for a turn.
            </DialogDescription>
          </DialogHeader>

          <div className="max-h-72 space-y-1 overflow-y-auto">
            {invitableAgents.length === 0 ? (
              <p className="py-6 text-center text-sm text-muted-foreground">
                No other agents available to invite.
              </p>
            ) : (
              invitableAgents.map((agent) => (
                <button
                  key={agent.id}
                  type="button"
                  disabled={addParticipant.isPending}
                  onClick={() => handleInvite(agent.id)}
                  className="flex w-full items-center gap-2 border border-transparent px-2 py-2 text-left text-sm transition-colors hover:border-border/70 hover:bg-muted disabled:opacity-50"
                >
                  <Bot className="h-4 w-4 flex-shrink-0 text-muted-foreground" />
                  <span className="truncate">{getDisplayName(agent)}</span>
                </button>
              ))
            )}
          </div>

          <DialogFooter>
            {addParticipant.isPending && (
              <span className="mr-auto inline-flex items-center gap-1.5 text-xs text-muted-foreground">
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                Inviting…
              </span>
            )}
            <Button type="button" variant="outline" onClick={() => setInviteOpen(false)}>
              Close
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </aside>
  );
}
