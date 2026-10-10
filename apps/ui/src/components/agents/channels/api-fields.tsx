"use client";

// Agent API (`api` channel) form fields and their state <-> config mapping.
// Kept out of channel-form.tsx, which is near the file-size limit. The config
// mirrors `AgentApiChannelConfig` in
// crates/server/src/domains/agent_channels/record/api.rs; limits match its
// validation (rate limit at most 1,000,000, activity text at most 200 chars).

import { useId } from "react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

export type ApiSessionBinding = "per_user" | "session_per_invocation";
export type ApiVisibility = "messages" | "activity" | "full";
export type ApiErrorDetail = "public" | "detailed";
export type ApiToolApprovals = "operator" | "caller";

export interface ApiChannelConfig {
  session_binding?: ApiSessionBinding;
  visibility?: ApiVisibility;
  errors?: ApiErrorDetail;
  tool_activity_text?: string;
  tool_approvals?: ApiToolApprovals;
  rate_limit_per_minute?: number;
}

export interface ApiFormState {
  sessionBinding: ApiSessionBinding;
  visibility: ApiVisibility;
  errors: ApiErrorDetail;
  toolActivityText: string;
  toolApprovals: ApiToolApprovals;
  /** Kept as text so the input can be edited freely; parsed on save. */
  rateLimitPerMinute: string;
}

const MAX_TOOL_ACTIVITY_TEXT = 200;
const MAX_RATE_LIMIT = 1_000_000;

export function apiFormStateFromConfig(config?: ApiChannelConfig): ApiFormState {
  return {
    sessionBinding: config?.session_binding ?? "per_user",
    visibility: config?.visibility ?? "activity",
    errors: config?.errors ?? "public",
    toolActivityText: config?.tool_activity_text ?? "",
    toolApprovals: config?.tool_approvals ?? "operator",
    rateLimitPerMinute:
      config?.rate_limit_per_minute != null ? String(config.rate_limit_per_minute) : "",
  };
}

/** `null` for invalid input, `undefined` for "no per-channel cap". */
function parseRateLimit(value: string): number | null | undefined {
  const trimmed = value.trim();
  if (!trimmed) return undefined;
  if (!/^\d+$/.test(trimmed)) return null;
  const parsed = Number.parseInt(trimmed, 10);
  return parsed <= MAX_RATE_LIMIT ? parsed : null;
}

export function buildApiChannelConfig(state: ApiFormState): ApiChannelConfig {
  const rateLimit = parseRateLimit(state.rateLimitPerMinute);
  const activityText = state.toolActivityText.trim();
  return {
    session_binding: state.sessionBinding,
    visibility: state.visibility,
    errors: state.errors,
    tool_approvals: state.toolApprovals,
    ...(activityText ? { tool_activity_text: activityText } : {}),
    ...(rateLimit ? { rate_limit_per_minute: rateLimit } : {}),
  };
}

export function isApiFormValid(state: ApiFormState): boolean {
  return (
    parseRateLimit(state.rateLimitPerMinute) !== null &&
    state.toolActivityText.trim().length <= MAX_TOOL_ACTIVITY_TEXT
  );
}

export function ApiFields({
  value,
  onChange,
}: {
  value: ApiFormState;
  onChange: (value: ApiFormState) => void;
}) {
  const id = useId();
  const set = <K extends keyof ApiFormState>(key: K, next: ApiFormState[K]) =>
    onChange({ ...value, [key]: next });
  const rateLimitValid = parseRateLimit(value.rateLimitPerMinute) !== null;

  return (
    <div className="space-y-4">
      <p className="text-xs text-muted-foreground">
        Your code calls this agent with an agent key. A key reaches only this agent&apos;s session
        routes and sees only the sessions it started.
      </p>
      <div className="grid gap-4 md:grid-cols-2">
        <div className="space-y-2">
          <Label htmlFor={`${id}_visibility`}>What callers see</Label>
          <Select
            value={value.visibility}
            onValueChange={(next) => set("visibility", next as ApiVisibility)}
          >
            <SelectTrigger id={`${id}_visibility`}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="messages">Messages only</SelectItem>
              <SelectItem value="activity">Messages and tool activity</SelectItem>
              <SelectItem value="full">Everything (raw events)</SelectItem>
            </SelectContent>
          </Select>
          <p className="text-xs text-muted-foreground">
            Tool activity shows that a tool ran, never its name or arguments. Everything is for
            callers who own both ends.
          </p>
        </div>
        <div className="space-y-2">
          <Label htmlFor={`${id}_tool_approvals`}>Who allows held-back tool calls</Label>
          <Select
            value={value.toolApprovals}
            onValueChange={(next) => set("toolApprovals", next as ApiToolApprovals)}
          >
            <SelectTrigger id={`${id}_tool_approvals`}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="operator">People with access to this agent</SelectItem>
              <SelectItem value="caller">The caller</SelectItem>
            </SelectContent>
          </Select>
          <p className="text-xs text-muted-foreground">
            The caller then sees the tool and its arguments, which it needs to decide.
          </p>
        </div>
        <div className="space-y-2">
          <Label htmlFor={`${id}_session_binding`}>Sessions</Label>
          <Select
            value={value.sessionBinding}
            onValueChange={(next) => set("sessionBinding", next as ApiSessionBinding)}
          >
            <SelectTrigger id={`${id}_session_binding`}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="per_user">One per caller conversation</SelectItem>
              <SelectItem value="session_per_invocation">One per created session</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-2">
          <Label htmlFor={`${id}_errors`}>Error detail</Label>
          <Select
            value={value.errors}
            onValueChange={(next) => set("errors", next as ApiErrorDetail)}
          >
            <SelectTrigger id={`${id}_errors`}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="public">Public error codes only</SelectItem>
              <SelectItem value="detailed">Detailed problem details</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-2">
          <Label htmlFor={`${id}_rate_limit`}>Rate limit / minute</Label>
          <Input
            id={`${id}_rate_limit`}
            value={value.rateLimitPerMinute}
            onChange={(event) => set("rateLimitPerMinute", event.target.value)}
            inputMode="numeric"
            placeholder="No per-channel cap"
            aria-invalid={!rateLimitValid}
          />
          {!rateLimitValid && (
            <p className="text-xs text-destructive">Use a whole number up to 1,000,000.</p>
          )}
        </div>
        {value.visibility === "activity" && (
          <div className="space-y-2">
            <Label htmlFor={`${id}_tool_activity_text`}>Tool activity text</Label>
            <Input
              id={`${id}_tool_activity_text`}
              value={value.toolActivityText}
              onChange={(event) => set("toolActivityText", event.target.value)}
              maxLength={MAX_TOOL_ACTIVITY_TEXT}
              placeholder="Working on it"
            />
          </div>
        )}
      </div>
    </div>
  );
}
