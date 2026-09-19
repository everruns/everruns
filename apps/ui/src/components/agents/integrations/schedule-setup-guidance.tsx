import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { getInvocationSessionModeDisplayName } from "@/lib/app-channels";
import type { InvocationSessionMode } from "@/lib/api/types";
import { Clock3 } from "lucide-react";

interface ScheduleSetupGuidanceProps {
  cronExpression: string;
  timezone: string;
  sessionMode: InvocationSessionMode;
  message: string;
  isEnabled: boolean;
  onConfigure?: () => void;
}

export function ScheduleSetupGuidance({
  cronExpression,
  timezone,
  sessionMode,
  message,
  isEnabled,
  onConfigure,
}: ScheduleSetupGuidanceProps) {
  return (
    <div className="space-y-4">
      <div className="flex items-center gap-2">
        <Badge variant={isEnabled ? "default" : "secondary"}>
          {isEnabled ? "Enabled" : "Disabled"}
        </Badge>
        <span className="text-sm text-muted-foreground">
          {isEnabled
            ? "This schedule enqueues agent invocations on its cron."
            : "Enable the trigger to activate this schedule."}
        </span>
      </div>

      <div>
        <p className="text-sm font-medium">Cron</p>
        <div className="mt-2 flex items-center gap-2 bg-muted p-3">
          <Clock3 className="h-4 w-4 shrink-0 text-muted-foreground" />
          <code className="flex-1 truncate text-sm">{cronExpression}</code>
          <Badge variant="outline">{timezone}</Badge>
        </div>
      </div>

      <div className="grid gap-4 md:grid-cols-2">
        <div>
          <p className="text-sm font-medium">Session Mode</p>
          <p className="text-sm text-muted-foreground">
            {getInvocationSessionModeDisplayName(sessionMode)}
          </p>
        </div>
        <div>
          <p className="text-sm font-medium">Invocation Message</p>
          <p className="text-sm text-muted-foreground">
            Sent as the user message when the schedule fires.
          </p>
        </div>
      </div>

      <div className="rounded-md border p-3 text-sm text-muted-foreground">
        <code className="whitespace-pre-wrap break-words">{message}</code>
      </div>

      <div className="space-y-1 text-sm text-muted-foreground">
        <p>Templates can reference agent and invocation fields.</p>
        <p>This is agent-level automation, not the in-session scheduler.</p>
      </div>

      {onConfigure && (
        <div className="pt-1">
          <Button size="sm" variant="outline" onClick={onConfigure}>
            Configure
          </Button>
        </div>
      )}
    </div>
  );
}
