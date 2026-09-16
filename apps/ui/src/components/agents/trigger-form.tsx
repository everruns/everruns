"use client";

import type { InvocationSessionMode } from "@/lib/api/types";
import { CronInput, isSupportedCronExpression } from "@/components/apps/cron-label";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";

export type TriggerConfig = {
  cron_expression: string;
  timezone?: string;
  session_mode?: InvocationSessionMode;
  message: string;
};

export type TriggerFormState = TriggerConfig & { enabled: boolean };

export const EMPTY_TRIGGER_FORM: TriggerFormState = {
  cron_expression: "0 0 9 * * * *",
  timezone: "UTC",
  session_mode: "shared_session",
  message: "",
  enabled: true,
};

export function isTriggerFormValid(form: TriggerFormState): boolean {
  return isSupportedCronExpression(form.cron_expression) && form.message.trim().length > 0;
}

/// The trigger's editable fields, shared by the inline panel and the full-page
/// route (EVE-1009). `CronInput` is the only place a raw cron expression is
/// shown — every read-only surface renders `CronLabel` instead.
export function TriggerFormFields({
  value,
  onChange,
  idPrefix = "trigger",
}: {
  value: TriggerFormState;
  onChange: (next: TriggerFormState) => void;
  idPrefix?: string;
}) {
  return (
    <>
      <CronInput
        value={value.cron_expression}
        timezone={value.timezone ?? "UTC"}
        onChange={(cron_expression) => onChange({ ...value, cron_expression })}
        onTimezoneChange={(timezone) => onChange({ ...value, timezone })}
      />
      <div className="space-y-2">
        <Label htmlFor={`${idPrefix}-session-mode`}>Session mode</Label>
        <Select
          value={value.session_mode}
          onValueChange={(session_mode) =>
            onChange({ ...value, session_mode: session_mode as InvocationSessionMode })
          }
        >
          <SelectTrigger id={`${idPrefix}-session-mode`} className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="shared_session">Shared session</SelectItem>
            <SelectItem value="session_per_invocation">New session per run</SelectItem>
          </SelectContent>
        </Select>
      </div>
      <div className="space-y-2">
        <Label htmlFor={`${idPrefix}-message`}>Message</Label>
        <Textarea
          id={`${idPrefix}-message`}
          value={value.message}
          onChange={(event) => onChange({ ...value, message: event.target.value })}
          placeholder="Run the daily digest"
          rows={4}
        />
      </div>
      <div className="flex items-center gap-2">
        <Switch
          id={`${idPrefix}-enabled`}
          checked={value.enabled}
          onCheckedChange={(enabled) => onChange({ ...value, enabled })}
        />
        <Label htmlFor={`${idPrefix}-enabled`}>Enabled</Label>
      </div>
    </>
  );
}
