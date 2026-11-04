"use client";

// What was done about a finding (approved mockup 04, "Done automatically"):
// a repeating schedule the org's self-healing policy paused, with Resume to
// undo it, or the pause offered for a person to take when the policy says
// "suggest" (knowledge/evaluation/self-healing.md).

import { Pause, Undo2 } from "lucide-react";
import { useUiTranslations } from "@/providers/locale-provider";
import { useFindingActionMutation, useFindingActions } from "@/hooks/use-evals";
import { usePolicies } from "@/hooks/use-policies";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { FindingAction, FindingActions } from "@/lib/api/types";

export function FindingActionsCard({ issueId }: { issueId: string }) {
  const { data } = useFindingActions(issueId);
  if (!data) return null;
  return <ActionsCard issueId={issueId} data={data} />;
}

function ActionsCard({ issueId, data }: { issueId: string; data: FindingActions }) {
  const ui = useUiTranslations();
  const { can } = usePolicies("agents");
  const mutation = useFindingActionMutation(issueId);
  const canManage = can("agent.manage");
  const current = data.actions.filter((action) => action.current);
  const offered = data.available.find((action) => action.kind === "pause_schedule");
  if (current.length === 0 && !offered) return null;

  const automatic = current.some((action) => action.automatic && !action.undone_at);
  const title = offered
    ? ui("Suggested action")
    : automatic
      ? ui("Done automatically")
      : ui("Done");

  return (
    <Card data-testid="finding-actions">
      <CardHeader>
        <CardTitle className="text-sm">{title}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-4 text-sm">
        {current.map((action) => (
          <ActionItem
            key={action.id}
            action={action}
            canUndo={canManage && !action.undone_at}
            busy={mutation.isPending}
            onUndo={() => mutation.mutate({ kind: "undo", actionId: action.id })}
          />
        ))}
        {offered && (
          <div className="space-y-2">
            <p className="flex items-center gap-2 font-medium">
              <Pause className="size-4" aria-hidden />
              {ui("Pause the schedule")}
            </p>
            <p className="text-muted-foreground">
              {ui(
                "Its runs keep ending the same way. Pausing stops the next runs; you can resume it from here.",
              )}
            </p>
            {canManage && (
              <Button
                size="sm"
                variant="outline"
                disabled={mutation.isPending}
                onClick={() => mutation.mutate({ kind: "take", actionKind: "pause_schedule" })}
              >
                <Pause className="size-4" aria-hidden />
                {ui("Pause schedule")}
              </Button>
            )}
          </div>
        )}
        {mutation.error && (
          <p className="text-destructive" role="alert">
            {mutation.error.message}
          </p>
        )}
      </CardContent>
    </Card>
  );
}

function ActionItem({
  action,
  canUndo,
  busy,
  onUndo,
}: {
  action: FindingAction;
  canUndo: boolean;
  busy: boolean;
  onUndo: () => void;
}) {
  const ui = useUiTranslations();
  if (action.undone_at) {
    return <p className="text-muted-foreground">{ui("The schedule was paused, then resumed.")}</p>;
  }
  return (
    <div className="space-y-2">
      <p className="flex items-center gap-2 font-medium">
        <Pause className="size-4" aria-hidden />
        {ui("Paused the schedule")}
      </p>
      <p className="text-muted-foreground">
        {action.automatic
          ? ui(
              "Your organization pauses repeating schedules automatically. The schedule's owner and the agent's authors were told.",
            )
          : ui("Paused from this finding.")}
      </p>
      {canUndo && (
        <Button size="sm" variant="outline" disabled={busy} onClick={onUndo}>
          <Undo2 className="size-4" aria-hidden />
          {ui("Resume schedule")}
        </Button>
      )}
    </div>
  );
}
