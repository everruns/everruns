"use client";

// How Slack reaches a channel: Socket Mode (Everruns connects out, no public
// address needed) or the Request URL (Slack calls this server over public
// HTTPS). Kept apart from channel-form.tsx, which is at the file size limit.

import { buttonVariants } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { SlackIcon as Slack } from "@/components/icons/slack-icon";
import { slackAppUrl } from "@/components/slack/slack-workspaces";
import { SlackWorkspaceName } from "./slack-install";
import { isPublicHttpsUrl } from "@/lib/public-origin";
import { useUiTranslations } from "@/providers/locale-provider";
import type { SlackConnection } from "@/lib/api/types";

/** Socket Mode unless this page is served from a public HTTPS address. */
export function defaultSlackConnection(): SlackConnection {
  if (typeof window === "undefined") return "socket_mode";
  return isPublicHttpsUrl(window.location.origin) ? "request_url" : "socket_mode";
}

const OPTIONS: { value: SlackConnection; label: string; hint: string }[] = [
  {
    value: "socket_mode",
    label: "Socket Mode",
    hint: "Everruns connects out to Slack. Works without a public address, such as on your own computer.",
  },
  {
    value: "request_url",
    label: "Request URL",
    hint: "Slack sends each message to this server. Needs a public HTTPS address.",
  },
];

export function SlackConnectionChoice({
  idPrefix,
  value,
  onChange,
}: {
  idPrefix: string;
  value: SlackConnection;
  onChange: (value: SlackConnection) => void;
}) {
  const ui = useUiTranslations();
  return (
    <fieldset className="space-y-2">
      <legend className="text-sm font-medium">{ui("How Slack reaches Everruns")}</legend>
      {OPTIONS.map((option) => (
        <label
          key={option.value}
          htmlFor={`${idPrefix}_connection_${option.value}`}
          className="flex cursor-pointer items-start gap-2 text-sm"
        >
          <input
            id={`${idPrefix}_connection_${option.value}`}
            type="radio"
            name={`${idPrefix}_connection`}
            className="mt-1"
            checked={value === option.value}
            onChange={() => onChange(option.value)}
          />
          <span>
            <span className="font-medium">{ui(option.label)}</span>
            <span className="block text-xs text-muted-foreground">{ui(option.hint)}</span>
          </span>
        </label>
      ))}
    </fieldset>
  );
}

export function SlackSecretsNote({ connection }: { connection: SlackConnection }) {
  const ui = useUiTranslations();
  return connection === "socket_mode"
    ? ui(
        "These fields are optional when saving. Everruns connects to Slack once an app-level token is saved; a bot token is needed to send replies.",
      )
    : ui(
        "These fields are optional when saving. Slack requests are rejected until a signing secret is set; a bot token is needed to send replies.",
      );
}

/** The secret the chosen connection needs: an app-level token or the signing secret. */
export function SlackSecretField({
  idPrefix,
  connection,
  mode,
  appId,
  appToken,
  signingSecret,
  onAppTokenChange,
  onSigningSecretChange,
}: {
  idPrefix: string;
  connection: SlackConnection;
  mode: "new" | "edit";
  appId: string;
  appToken: string;
  signingSecret: string;
  onAppTokenChange: (value: string) => void;
  onSigningSecretChange: (value: string) => void;
}) {
  const ui = useUiTranslations();
  if (connection === "socket_mode") {
    return (
      <div className="space-y-2">
        <Label htmlFor={`${idPrefix}_app_token`}>{ui("App-level token")}</Label>
        <Input
          id={`${idPrefix}_app_token`}
          type="password"
          aria-describedby={`${idPrefix}_app_token_description`}
          value={appToken}
          onChange={(event) => onAppTokenChange(event.target.value)}
          placeholder={mode === "edit" ? ui("Leave blank to keep existing token") : "xapp-..."}
        />
        <p
          id={`${idPrefix}_app_token_description`}
          className="text-xs leading-relaxed text-muted-foreground"
        >
          {ui(
            "In your Slack app, open Basic Information, then App-Level Tokens, and generate one with the connections:write scope.",
          )}{" "}
          {appId && (
            <a
              className="underline"
              href={`https://api.slack.com/apps/${appId}/general`}
              target="_blank"
              rel="noreferrer"
            >
              {ui("Open the app's settings in Slack")}
            </a>
          )}
        </p>
      </div>
    );
  }
  return (
    <div className="space-y-2">
      <Label htmlFor={`${idPrefix}_signing_secret`}>{ui("Signing secret")}</Label>
      <Input
        id={`${idPrefix}_signing_secret`}
        type="password"
        value={signingSecret}
        onChange={(event) => onSigningSecretChange(event.target.value)}
        placeholder={
          mode === "edit" ? ui("Leave blank to keep existing secret") : ui("Slack signing secret")
        }
      />
    </div>
  );
}

/** The agent already has its Slack app: where it lives and a link to open it. */
export function SlackInstalledCard({
  showTitle,
  appId,
  teamId,
  agentSurfaceEnabled,
}: {
  showTitle: boolean;
  appId: string;
  teamId: string;
  agentSurfaceEnabled: boolean;
}) {
  const ui = useUiTranslations();
  return (
    <div className="space-y-3 border p-4">
      <div className="space-y-1">
        {showTitle && <p className="text-sm font-medium">{ui("Installed in Slack")}</p>}
        <p className="text-xs text-muted-foreground">
          {ui("This agent has its own Slack app in workspace")}{" "}
          <SlackWorkspaceName teamId={teamId} />
          {ui(". Mention it in a channel")}
          {agentSurfaceEnabled ? ui(", or open it from Slack’s Agents menu.") : "."}
        </p>
      </div>
      <a
        className={buttonVariants({ variant: "outline", size: "sm" })}
        href={slackAppUrl(appId, teamId)}
        target="_blank"
        rel="noreferrer"
      >
        <Slack className="size-4" />
        {ui("Open in Slack")}
      </a>
    </div>
  );
}
