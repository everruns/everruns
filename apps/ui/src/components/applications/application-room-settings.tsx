"use client";
import { useState } from "react";
import { useAgents } from "@/hooks/use-agents";
import { useOrgScopedQuery } from "@/hooks/create-crud-hooks";
import { addRoomMember, listRoomMembers, updateRoomMember, type WakeMode } from "@/lib/api/rooms";
import { api } from "@/lib/api/client";
import { useWakeModeLabel } from "@/components/rooms/room-parts";
import { Button } from "@/components/ui/button";
import { useUiTranslations } from "@/providers/locale-provider";

export function ApplicationRoomSettings({ roomId }: { roomId: string }) {
  const ui = useUiTranslations();
  const wakeLabel = useWakeModeLabel();
  const agents = useAgents();
  const members = useOrgScopedQuery({
    queryKey: ["application-room-members", roomId],
    queryFn: () => listRoomMembers(roomId, false),
  });
  const [agent, setAgent] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  async function change(action: () => Promise<unknown>) {
    setBusy(true);
    setError("");
    try {
      await action();
      await members.refetch();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <details className="border-t border-border pt-3 text-sm">
      <summary className="cursor-pointer font-medium">{ui("Room members")}</summary>
      {error && (
        <p role="alert" className="mt-2 text-destructive">
          {error}
        </p>
      )}
      <div className="mt-3 space-y-3">
        {(members.data ?? []).map((member) => (
          <div key={member.id} className="flex flex-wrap items-center gap-2">
            <span className="min-w-0 flex-1 truncate">
              {member.display_name} <span className="text-muted-foreground">@{member.handle}</span>
            </span>
            {member.kind === "agent" && (
              <select
                aria-label={ui("Wake mode")}
                className="h-9 border border-input bg-background px-2"
                value={member.wake}
                disabled={busy}
                onChange={(e) =>
                  void change(() =>
                    updateRoomMember(roomId, member.id, { wake: e.target.value as WakeMode }),
                  )
                }
              >
                {(["mentions", "everything", "relevant", "never"] as const).map((wake) => (
                  <option key={wake} value={wake}>
                    {wakeLabel(wake)}
                  </option>
                ))}
              </select>
            )}
            <Button
              size="sm"
              variant="ghost"
              disabled={busy}
              onClick={() => {
                if (confirm(ui("Remove this room member?")))
                  void change(() =>
                    api.delete(`/v1/platform/rooms/${roomId}/members/${member.id}`),
                  );
              }}
            >
              {ui("Remove")}
            </Button>
          </div>
        ))}
        <div className="flex gap-2">
          <select
            aria-label={ui("Select an agent")}
            className="h-9 min-w-0 flex-1 border border-input bg-background px-2"
            value={agent}
            onChange={(e) => setAgent(e.target.value)}
          >
            <option value="">{ui("Select an agent")}</option>
            {(agents.data ?? [])
              .filter((a) => !(members.data ?? []).some((m) => m.agent_id === a.id))
              .map((a) => (
                <option key={a.id} value={a.id}>
                  {a.name}
                </option>
              ))}
          </select>
          <Button
            size="sm"
            disabled={busy || !agent}
            onClick={() =>
              void change(async () => {
                await addRoomMember(roomId, { kind: "agent", agent_id: agent, wake: "mentions" });
                setAgent("");
              })
            }
          >
            {ui("Add agent")}
          </Button>
        </div>
      </div>
    </details>
  );
}
