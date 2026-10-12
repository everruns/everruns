"use client";

// Agent API (`api` channel) form fields and their state <-> config mapping.
// Kept out of channel-form.tsx, which is near the file-size limit. The config
// mirrors `AgentApiChannelConfig` in
// crates/server/src/domains/agent_channels/record/api.rs; limits match its
// validation (rate limit at most 1,000,000, activity text at most 200 chars,
// at most 20 browser origins, each exactly as a browser sends it, a daily
// spending limit above 0 and at most 1,000,000 dollars).
//
// `auth_methods` (the customer's identity providers) has no form yet: it is
// carried through unchanged so saving this form never drops it.

import { useId } from "react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
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
  auth_methods?: unknown[];
  cors_origins?: string[];
  daily_spend_limit_usd?: number;
  org_members?: boolean;
}

export interface ApiFormState {
  sessionBinding: ApiSessionBinding;
  visibility: ApiVisibility;
  errors: ApiErrorDetail;
  toolActivityText: string;
  toolApprovals: ApiToolApprovals;
  /** Kept as text so the input can be edited freely; parsed on save. */
  rateLimitPerMinute: string;
  /** One origin per line. */
  corsOrigins: string;
  /** Kept as text; parsed on save. Empty means no limit. */
  dailySpendLimitUsd: string;
  orgMembers: boolean;
  authMethods: unknown[];
}

const MAX_TOOL_ACTIVITY_TEXT = 200;
const MAX_RATE_LIMIT = 1_000_000;
const MAX_CORS_ORIGINS = 20;
const MAX_DAILY_SPEND_LIMIT = 1_000_000;

export function apiFormStateFromConfig(config?: ApiChannelConfig): ApiFormState {
  return {
    sessionBinding: config?.session_binding ?? "per_user",
    visibility: config?.visibility ?? "activity",
    errors: config?.errors ?? "public",
    toolActivityText: config?.tool_activity_text ?? "",
    toolApprovals: config?.tool_approvals ?? "operator",
    rateLimitPerMinute:
      config?.rate_limit_per_minute != null ? String(config.rate_limit_per_minute) : "",
    corsOrigins: (config?.cors_origins ?? []).join("\n"),
    dailySpendLimitUsd:
      config?.daily_spend_limit_usd != null ? String(config.daily_spend_limit_usd) : "",
    orgMembers: config?.org_members ?? false,
    authMethods: config?.auth_methods ?? [],
  };
}

/** `null` for invalid input, `undefined` for "no limit". */
function parseSpendLimit(value: string): number | null | undefined {
  const trimmed = value.trim();
  if (!trimmed) return undefined;
  if (!/^\d+(\.\d+)?$/.test(trimmed)) return null;
  const parsed = Number.parseFloat(trimmed);
  return parsed > 0 && parsed <= MAX_DAILY_SPEND_LIMIT ? parsed : null;
}

function corsOriginList(value: string): string[] {
  return value
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
}

/** An origin as a browser sends it: https (http only for loopback), no path. */
function isBrowserOrigin(value: string): boolean {
  try {
    const url = new URL(value);
    const loopback = ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname);
    const schemeOk = url.protocol === "https:" || (url.protocol === "http:" && loopback);
    return schemeOk && url.origin === value;
  } catch {
    return false;
  }
}

function corsOriginsValid(value: string): boolean {
  const origins = corsOriginList(value);
  return origins.length <= MAX_CORS_ORIGINS && origins.every(isBrowserOrigin);
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
  const spendLimit = parseSpendLimit(state.dailySpendLimitUsd);
  const activityText = state.toolActivityText.trim();
  return {
    session_binding: state.sessionBinding,
    visibility: state.visibility,
    errors: state.errors,
    tool_approvals: state.toolApprovals,
    ...(activityText ? { tool_activity_text: activityText } : {}),
    ...(rateLimit ? { rate_limit_per_minute: rateLimit } : {}),
    ...(spendLimit ? { daily_spend_limit_usd: spendLimit } : {}),
    ...(state.orgMembers ? { org_members: true } : {}),
    ...(state.authMethods.length ? { auth_methods: state.authMethods } : {}),
    ...(corsOriginList(state.corsOrigins).length
      ? { cors_origins: corsOriginList(state.corsOrigins) }
      : {}),
  };
}

export function isApiFormValid(state: ApiFormState): boolean {
  return (
    parseRateLimit(state.rateLimitPerMinute) !== null &&
    state.toolActivityText.trim().length <= MAX_TOOL_ACTIVITY_TEXT &&
    corsOriginsValid(state.corsOrigins) &&
    parseSpendLimit(state.dailySpendLimitUsd) !== null
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
  const originsValid = corsOriginsValid(value.corsOrigins);
  const spendLimitValid = parseSpendLimit(value.dailySpendLimitUsd) !== null;

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
        <div className="space-y-2">
          <Label htmlFor={`${id}_daily_spend_limit`}>Daily spending limit per caller (USD)</Label>
          <Input
            id={`${id}_daily_spend_limit`}
            value={value.dailySpendLimitUsd}
            onChange={(event) => set("dailySpendLimitUsd", event.target.value)}
            inputMode="decimal"
            placeholder="No limit"
            aria-invalid={!spendLimitValid}
          />
          {!spendLimitValid && (
            <p className="text-xs text-destructive">Use an amount above 0, up to 1,000,000.</p>
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
      <div className="flex items-center justify-between gap-4">
        <div>
          <Label htmlFor={`${id}_org_members`}>Organization members</Label>
          <p className="text-xs text-muted-foreground">
            Members may call this agent with their own personal access token, using their own
            connections.
          </p>
        </div>
        <Switch
          id={`${id}_org_members`}
          checked={value.orgMembers}
          onCheckedChange={(checked) => set("orgMembers", checked)}
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor={`${id}_cors_origins`}>Browser origins</Label>
        <Textarea
          id={`${id}_cors_origins`}
          value={value.corsOrigins}
          onChange={(event) => set("corsOrigins", event.target.value)}
          placeholder="https://app.example.com"
          rows={3}
          aria-invalid={!originsValid}
        />
        <p className="text-xs text-muted-foreground">
          One per line. Web pages on these origins may call this agent directly with a runtime
          token. Never put an agent key in a browser.
        </p>
        {!originsValid && (
          <p className="text-xs text-destructive">
            Use up to {MAX_CORS_ORIGINS} origins like https://app.example.com, with no path or
            trailing slash (http only for localhost).
          </p>
        )}
      </div>
    </div>
  );
}
