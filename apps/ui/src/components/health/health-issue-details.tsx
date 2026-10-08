"use client";

import { useState } from "react";
import Link from "next/link";
import { useQuery } from "@tanstack/react-query";
import { AlertTriangle, CheckCircle2, ExternalLink } from "lucide-react";
import { Button, LinkButton } from "@/components/ui/button";
import { useAuth } from "@/providers/auth-provider";
import { useOrg } from "@/providers/org-provider";
import { useHealthIssueAction } from "@/hooks/use-health-issues";
import { getAgentChannel, beginSlackInstall } from "@/lib/api/agent-channels";
import type { HealthIssue, SlackChannelConfig } from "@/lib/api/types";

const SCOPE_LABELS: Record<string, string> = {
  "reactions:write": "Add and remove emoji reactions",
  "chat:write": "Send messages as this bot",
  "assistant:write": "Use Slack's agent surface",
  "channels:history": "Read messages in public channels this bot has joined",
  "groups:history": "Read messages in private channels this bot has joined",
  "im:history": "Read direct messages with this bot",
  "mpim:history": "Read group direct messages with this bot",
  "app_mentions:read": "Read messages that mention this bot",
  "users:read": "Read workspace member information",
  "files:read": "Read files shared with this bot",
};

export function HealthIssueDetails({
  issue,
  canManage = true,
}: {
  issue: HealthIssue;
  canManage?: boolean;
}) {
  const { currentOrg } = useOrg();
  const { requiresAuth, user } = useAuth();
  const check = useHealthIssueAction("check");
  const snooze = useHealthIssueAction("snooze");
  const [connecting, setConnecting] = useState(false);
  const [connectError, setConnectError] = useState<string | null>(null);
  // An organization-level issue (the active turn limit) has no agent or channel.
  const orgIssue = !issue.agent_id || !issue.channel_id;
  const channel = useQuery({
    queryKey: ["health-channel", currentOrg?.public_id, issue.channel_id],
    queryFn: () => getAgentChannel(issue.agent_id ?? "", issue.channel_id ?? ""),
    enabled: canManage && !orgIssue,
    retry: false,
  });
  const config = channel.data?.channel_config as
    | (SlackChannelConfig & { slack_app_provisioned?: boolean })
    | undefined;
  const resolved = issue.status === "resolved" || issue.status === "inapplicable";
  const Icon = resolved ? CheckCircle2 : AlertTriangle;
  const integrationHref = `/agents/${issue.agent_id ?? ""}/channels/${issue.channel_id ?? ""}`;
  return (
    <section
      className="space-y-4 border border-border bg-card p-4"
      aria-label="Health issue details"
    >
      <div className="flex items-start gap-3">
        <Icon className={`mt-0.5 size-4 shrink-0 ${resolved ? "text-success" : "text-warning"}`} />
        <div className="min-w-0 space-y-1">
          <h3 className="text-base font-medium">{issue.title}</h3>
          <p className="text-xs text-muted-foreground">
            {issue.agent_name ?? "Organization"}
            {config?.team_id ? ` · ${config.team_id}` : ""}
          </p>
          <p className="text-sm">{issue.body}</p>
        </div>
      </div>
      {issue.stale && (
        <p className="text-sm text-warning">
          Verification is unavailable or out of date.{" "}
          {resolved
            ? "Run a fresh check before relying on this previous result."
            : "The issue remains open until a fresh check succeeds."}
        </p>
      )}
      {resolved && issue.stale && canManage && (
        <>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={check.isPending}
            onClick={() => check.mutate(issue.id)}
          >
            {check.isPending ? "Checking…" : "Check again"}
          </Button>
          {check.error && (
            <p role="alert" className="text-sm text-destructive">
              {check.error.message}
            </p>
          )}
        </>
      )}
      {!resolved && issue.missing_scopes.length > 0 && (
        <div className="space-y-2 bg-muted p-3">
          <p className="text-sm font-medium">Additional permissions</p>
          <ul className="space-y-2">
            {issue.missing_scopes.map((scope) => (
              <li key={scope} className="flex flex-wrap items-baseline gap-x-2 gap-y-1 text-sm">
                <span>{SCOPE_LABELS[scope] ?? scope}</span>
                <code className="max-w-full break-all text-xs text-muted-foreground">{scope}</code>
              </li>
            ))}
          </ul>
        </div>
      )}
      {!resolved && orgIssue && canManage && (
        <div className="flex flex-wrap gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={check.isPending}
            onClick={() => check.mutate(issue.id)}
          >
            {check.isPending ? "Checking…" : "Check again"}
          </Button>
        </div>
      )}
      {!resolved && !orgIssue && (
        <>
          <p className="text-xs text-muted-foreground">
            Review and approve the permissions in Slack. Your workspace may require administrator
            approval. Updating app settings alone does not grant permissions to an installed bot.
          </p>
          {canManage ? (
            <div className="flex flex-wrap gap-2">
              {config?.slack_app_provisioned ? (
                <Button
                  type="button"
                  size="sm"
                  disabled={connecting || check.isPending}
                  onClick={async () => {
                    setConnecting(true);
                    setConnectError(null);
                    try {
                      const result = await beginSlackInstall(
                        issue.channel_id ?? "",
                        config.team_id,
                      );
                      // This URL is generated by our authenticated install route.
                      const url = new URL(result.authorize_url);
                      if (
                        url.origin !== "https://slack.com" ||
                        url.pathname !== "/oauth/v2/authorize"
                      )
                        throw new Error("Invalid Slack authorization URL");
                      window.location.assign(url.href);
                    } catch (error) {
                      setConnectError(
                        error instanceof Error ? error.message : "Could not reconnect Slack",
                      );
                      setConnecting(false);
                    }
                  }}
                >
                  {connecting ? "Opening Slack…" : "Reconnect Slack"}
                  <ExternalLink className="size-3" />
                </Button>
              ) : (
                <LinkButton href={integrationHref} size="sm">
                  Open integration settings
                </LinkButton>
              )}
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={check.isPending || connecting}
                onClick={() => check.mutate(issue.id)}
              >
                {check.isPending ? "Checking…" : "Check again"}
              </Button>
            </div>
          ) : (
            <p className="text-sm">An administrator needs to reconnect this integration.</p>
          )}
          {canManage && !config?.slack_app_provisioned && (
            <p className="text-xs text-muted-foreground">
              For a manually configured app: add the listed bot scopes in Slack’s OAuth &amp;
              Permissions settings, reinstall the existing app, and save the refreshed bot token in
              Everruns.
            </p>
          )}
          {connectError && (
            <p role="alert" className="text-sm text-destructive">
              {connectError}{" "}
              <Link className="underline" href={integrationHref}>
                Configure manually
              </Link>
            </p>
          )}
          {check.error && (
            <p role="alert" className="text-sm text-destructive">
              {check.error.message}
            </p>
          )}
          {requiresAuth && user && (
            <div className="flex flex-wrap items-center gap-3">
              <Button
                type="button"
                size="sm"
                variant="ghost"
                disabled={snooze.isPending || !!issue.snoozed_until}
                onClick={() => snooze.mutate(issue.id)}
              >
                {issue.snoozed_until ? "Reminder snoozed" : "Remind me tomorrow"}
              </Button>
              {issue.snoozed_until && (
                <p className="text-xs text-muted-foreground">
                  Snoozed until {new Date(issue.snoozed_until).toLocaleString()}. The issue remains
                  open.
                </p>
              )}
            </div>
          )}
          {snooze.error && (
            <p role="alert" className="text-sm text-destructive">
              {snooze.error.message}
            </p>
          )}
        </>
      )}
      {!resolved && orgIssue && (
        <>
          {check.error && (
            <p role="alert" className="text-sm text-destructive">
              {check.error.message}
            </p>
          )}
          {requiresAuth && user && (
            <Button
              type="button"
              size="sm"
              variant="ghost"
              disabled={snooze.isPending || !!issue.snoozed_until}
              onClick={() => snooze.mutate(issue.id)}
            >
              {issue.snoozed_until ? "Reminder snoozed" : "Remind me tomorrow"}
            </Button>
          )}
        </>
      )}
      <p className="text-xs text-muted-foreground">
        Detected {new Date(issue.first_detected_at).toLocaleString()} · Last checked{" "}
        {new Date(issue.last_checked_at).toLocaleString()}
      </p>
      <details className="text-xs text-muted-foreground">
        <summary>Technical details</summary>
        <p className="mt-2">
          {issue.code}
          {issue.error_code ? ` · ${issue.error_code}` : ""}
        </p>
      </details>
    </section>
  );
}
