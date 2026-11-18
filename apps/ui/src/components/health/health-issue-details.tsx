"use client";

import { useUiTranslations, useLocale } from "@/providers/locale-provider";
import Link from "next/link";
import { useQuery } from "@tanstack/react-query";
import { AlertTriangle, CheckCircle2 } from "lucide-react";
import { Button, LinkButton } from "@/components/ui/button";
import { useAuth } from "@/providers/auth-provider";
import { useOrg } from "@/providers/org-provider";
import { useHealthIssueAction } from "@/hooks/use-health-issues";
import { getAgentChannel } from "@/lib/api/agent-channels";
import { HealthFixer } from "@/components/health/health-fixer";
import type { HealthIssue, SlackChannelConfig } from "@/lib/api/types";

const SCOPE_LABELS: Record<string, string> = {
  "reactions:write": "Add and remove emoji reactions",
  "reactions:read": "See emoji reactions people add",
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
  const { displayLocale } = useLocale();

  const ui = useUiTranslations();
  const { currentOrg } = useOrg();
  const { requiresAuth, user } = useAuth();
  const check = useHealthIssueAction("check");
  const snooze = useHealthIssueAction("snooze");
  // An organization-level issue (the active turn limit) has no agent or channel.
  const orgIssue = !issue.agent_id || !issue.channel_id;
  const channel = useQuery({
    queryKey: ["health-channel", currentOrg?.public_id, issue.channel_id],
    queryFn: () => getAgentChannel(issue.agent_id ?? "", issue.channel_id ?? ""),
    enabled: canManage && !orgIssue,
    retry: false,
  });
  const config = channel.data?.channel_config as
    | (SlackChannelConfig & { slack_app_provisioned?: boolean; slack_app_id?: string })
    | undefined;
  const resolved = issue.status === "resolved" || issue.status === "inapplicable";
  const Icon = resolved ? CheckCircle2 : AlertTriangle;
  const integrationHref = `/agents/${issue.agent_id ?? ""}/channels/${issue.channel_id ?? ""}`;
  const fixers = issue.fixers ?? [];
  const provisioned = !!config?.slack_app_provisioned;
  // Steps that work for every Slack app, whoever made it: Slack itself is the
  // one place a scope is added and approved, so these never need Everruns.
  const manualSlack = (
    <SlackManualSteps
      scopes={issue.missing_scopes}
      appId={config?.slack_app_id}
      provisioned={provisioned}
      integrationHref={integrationHref}
    />
  );
  return (
    <section
      className="space-y-4 border border-border bg-card p-4"
      aria-label={ui("Health issue details")}
    >
      <div className="flex items-start gap-3">
        <Icon className={`mt-0.5 size-4 shrink-0 ${resolved ? "text-success" : "text-warning"}`} />
        <div className="min-w-0 space-y-1">
          <h3 className="text-base font-medium">{issue.title}</h3>
          <p className="text-xs text-muted-foreground">
            {issue.agent_name ?? ui("Organization")}
            {config?.team_id ? ui(" · {value0}", { value0: config.team_id }) : ""}
          </p>
          <p className="text-sm">{issue.body}</p>
        </div>
      </div>
      {issue.stale && (
        <p className="text-sm text-warning">
          {ui("Verification is unavailable or out of date.")}{" "}
          {resolved
            ? ui("Run a fresh check before relying on this previous result.")
            : ui("The issue remains open until a fresh check succeeds.")}
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
            {check.isPending ? ui("Checking…") : ui("Check again")}
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
          <p className="text-sm font-medium">{ui("Additional permissions")}</p>
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
      {!resolved && canManage && fixers.length > 0 && (
        <div className="space-y-3">
          {fixers.map((fixer) => (
            <HealthFixer
              key={fixer.kind}
              issue={issue}
              fixer={fixer}
              manualSteps={
                !orgIssue && fixer.kind.startsWith("slack.") && issue.missing_scopes.length > 0
                  ? manualSlack
                  : undefined
              }
            />
          ))}
        </div>
      )}
      {!resolved && orgIssue && canManage && fixers.length === 0 && (
        <div className="flex flex-wrap gap-2">
          {issue.session_id && (
            <LinkButton href={`/sessions/${issue.session_id}`} size="sm">
              {ui("Open session")}
            </LinkButton>
          )}
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={check.isPending}
            onClick={() => check.mutate(issue.id)}
          >
            {check.isPending ? ui("Checking…") : ui("Check again")}
          </Button>
        </div>
      )}
      {!resolved && orgIssue && canManage && fixers.length > 0 && issue.session_id && (
        <LinkButton href={`/sessions/${issue.session_id}`} size="sm" variant="outline">
          {ui("Open session")}
        </LinkButton>
      )}
      {!resolved && !orgIssue && (
        <>
          {canManage ? (
            fixers.length === 0 && (
              <>
                <p className="text-xs text-muted-foreground">
                  {ui(
                    "Review and approve the permissions in Slack. Your workspace may require administrator approval. Updating app settings alone does not grant permissions to an installed bot.",
                  )}
                </p>
                <div className="flex flex-wrap gap-2">
                  <LinkButton href={integrationHref} size="sm">
                    {ui("Open integration settings")}
                  </LinkButton>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    disabled={check.isPending}
                    onClick={() => check.mutate(issue.id)}
                  >
                    {check.isPending ? ui("Checking…") : ui("Check again")}
                  </Button>
                </div>
                {issue.missing_scopes.length > 0 && manualSlack}
              </>
            )
          ) : (
            <p className="text-sm">{ui("An administrator needs to reconnect this integration.")}</p>
          )}
          {check.error && fixers.length === 0 && (
            <p role="alert" className="text-sm text-destructive">
              {check.error.message}
            </p>
          )}
        </>
      )}
      {!resolved && (
        <>
          {orgIssue && check.error && (
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
                {issue.snoozed_until ? ui("Reminder snoozed") : ui("Remind me tomorrow")}
              </Button>
              {issue.snoozed_until && (
                <p className="text-xs text-muted-foreground">
                  {ui("Snoozed until")}{" "}
                  {new Date(issue.snoozed_until).toLocaleString(displayLocale)}
                  {ui(". The issue remains open.")}
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
      <p className="text-xs text-muted-foreground">
        {ui("Detected")} {new Date(issue.first_detected_at).toLocaleString(displayLocale)}{" "}
        {ui("· Last checked")} {new Date(issue.last_checked_at).toLocaleString(displayLocale)}
      </p>
      <details className="text-xs text-muted-foreground">
        <summary>{ui("Technical details")}</summary>
        <p className="mt-2">
          {issue.code}
          {issue.error_code ? ui(" · {value0}", { value0: issue.error_code }) : ""}
        </p>
      </details>
    </section>
  );
}

/**
 * Adding Slack bot scopes by hand, for any app: Everruns-created apps keep
 * their bot token on reinstall; a hand-made app may get a new one to paste.
 */
function SlackManualSteps({
  scopes,
  appId,
  provisioned,
  integrationHref,
}: {
  scopes: string[];
  appId?: string | null;
  provisioned: boolean;
  integrationHref: string;
}) {
  const ui = useUiTranslations();
  const validAppId = appId && /^A[A-Z0-9]+$/.test(appId) ? appId : null;
  const appHref = validAppId
    ? `https://api.slack.com/apps/${validAppId}/oauth`
    : "https://api.slack.com/apps";
  return (
    <ol className="list-decimal space-y-1 pl-5 text-xs text-muted-foreground">
      <li>
        <a className="underline" href={appHref} target="_blank" rel="noopener noreferrer">
          {validAppId ? ui("Open the app in Slack") : ui("Open your Slack apps")}
        </a>
        {validAppId
          ? ui(" and go to OAuth & Permissions.")
          : ui(", choose this agent's app, and go to OAuth & Permissions.")}
      </li>
      <li>
        {ui("Under Bot Token Scopes, add")}{" "}
        {scopes.map((scope, i) => (
          <span key={scope}>
            {i > 0 && ", "}
            <code>{scope}</code>
          </span>
        ))}
        .
      </li>
      <li>
        {ui(
          "Click Reinstall to Workspace in the banner at the top and approve. Your workspace may need an administrator to approve it.",
        )}
      </li>
      <li>
        {provisioned ? (
          ui("The bot token stays the same, so there is nothing to paste back.")
        ) : (
          <>
            {ui("If Slack shows a new Bot User OAuth Token, save it in")}{" "}
            <Link className="underline" href={integrationHref}>
              {ui("integration settings")}
            </Link>
            .
          </>
        )}
      </li>
      <li>{ui("Come back here and click Check again.")}</li>
    </ol>
  );
}
