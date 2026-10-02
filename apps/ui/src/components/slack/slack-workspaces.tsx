"use client";

/**
 * Connecting Slack workspaces to an organization (EVE-1148).
 *
 * Decisions:
 * - Connecting a workspace is organization setup, done once by an admin; builders only pick from
 *   the connected list. Without Slack partner status a configuration token cannot be obtained by
 *   OAuth, so the wizard's job is to make the one manual step short and hard to get wrong.
 * - The token connects as soon as a refresh-shaped value is pasted. The access token sits right
 *   above the refresh token on Slack's page and is the obvious thing to copy by mistake, so it is
 *   named specifically rather than reported as a generic rejection after a round trip.
 */
import { useEffect, useRef, useState } from "react";
import { CircleAlert, ExternalLink } from "lucide-react";
import { SlackIcon as Slack } from "@/components/icons/slack-icon";
import { Button, buttonVariants } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { connectSlackWorkspace, type SlackWorkspace } from "@/lib/api/agent-endpoints";

/** Where Slack's "Your App Configuration Tokens" section lives. */
export const SLACK_APPS_URL = "https://api.slack.com/apps";

export type SlackConfigTokenShape = "empty" | "refresh" | "access" | "unknown";

/**
 * Configuration refresh tokens start `xoxe-1-`; the access token issued alongside starts
 * `xoxe.xoxp-`. Anything else is not a configuration token at all.
 */
export function classifySlackConfigToken(value: string): SlackConfigTokenShape {
  const token = value.trim();
  if (!token) return "empty";
  if (token.startsWith("xoxe-1-")) return "refresh";
  if (token.startsWith("xoxe.xoxp-")) return "access";
  return "unknown";
}

export function slackWorkspaceLabel(workspace: SlackWorkspace): string {
  if (workspace.team_name && workspace.team_id) {
    return `${workspace.team_name} (${workspace.team_id})`;
  }
  return workspace.team_name ?? workspace.team_id ?? "Identifying workspace…";
}

/** Opens the agent's app in Slack (its App Home), in the right workspace. */
export function slackAppUrl(appId: string, teamId?: string | null): string {
  const params = new URLSearchParams({ app: appId });
  if (teamId) params.set("team", teamId);
  return `https://slack.com/app_redirect?${params.toString()}`;
}

const SHAPE_MESSAGES: Partial<Record<SlackConfigTokenShape, string>> = {
  access:
    "That is the access token (it starts xoxe.xoxp-). Copy the refresh token shown below it on the same page.",
  unknown: "That does not look like a Slack configuration refresh token. It starts xoxe-1-.",
};

export function ConnectSlackWorkspace({
  reconnect = false,
  onConnected,
}: {
  reconnect?: boolean;
  onConnected?: (workspace: SlackWorkspace) => void | Promise<unknown>;
}) {
  const [token, setToken] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const attempted = useRef<string | null>(null);
  const shape = classifySlackConfigToken(token);

  const connect = async (value: string) => {
    attempted.current = value;
    setPending(true);
    setError(null);
    try {
      const workspace = await connectSlackWorkspace(value);
      setToken("");
      await onConnected?.(workspace);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Could not connect the Slack workspace.");
    } finally {
      setPending(false);
    }
  };

  // Connect on paste: once the value is refresh-shaped there is nothing left for the admin to do.
  // A refresh token is single use, so the same value is never submitted twice.
  useEffect(() => {
    const value = token.trim();
    if (shape === "refresh" && !pending && attempted.current !== value) void connect(value);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [token, shape, pending]);

  return (
    <div className="space-y-4">
      <ol className="space-y-3 text-sm">
        <li className="flex gap-3">
          <StepNumber n={1} />
          <div className="space-y-2">
            <p>
              Open Slack&apos;s app settings, signed in to the workspace you want your agents in.
            </p>
            <a
              href={SLACK_APPS_URL}
              target="_blank"
              rel="noreferrer"
              className={buttonVariants({ variant: "outline", size: "sm" })}
            >
              <Slack className="size-4" />
              Open api.slack.com/apps
              <ExternalLink className="size-3.5" />
            </a>
          </div>
        </li>
        <li className="flex gap-3">
          <StepNumber n={2} />
          <p>
            Scroll to <strong>Your App Configuration Tokens</strong>, click{" "}
            <strong>Generate Token</strong> and pick the workspace.
          </p>
        </li>
        <li className="flex gap-3">
          <StepNumber n={3} />
          <div className="w-full space-y-2">
            <Label htmlFor="slack_configuration_refresh_token">
              Copy the <strong>Refresh Token</strong> and paste it here
            </Label>
            <Input
              id="slack_configuration_refresh_token"
              type="password"
              autoComplete="off"
              value={token}
              onChange={(event) => setToken(event.target.value)}
              placeholder="xoxe-1-..."
              disabled={pending}
              aria-invalid={Boolean(SHAPE_MESSAGES[shape]) || Boolean(error)}
            />
            {SHAPE_MESSAGES[shape] && (
              <p className="text-xs text-destructive">{SHAPE_MESSAGES[shape]}</p>
            )}
            {pending && <p className="text-xs text-muted-foreground">Connecting…</p>}
            {error && <p className="text-xs text-destructive">{error}</p>}
            {error && shape === "refresh" && (
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => void connect(token.trim())}
              >
                {reconnect ? "Try reconnecting again" : "Try again"}
              </Button>
            )}
          </div>
        </li>
      </ol>
      <div className="flex gap-2 border border-warning/40 bg-warning/10 p-3 text-xs">
        <CircleAlert className="mt-0.5 size-4 shrink-0 text-warning" />
        <p>
          This token can create and change any Slack app in that workspace. Everruns rotates it
          immediately, stores only the encrypted replacement, and uses it only to create and update
          your agents&apos; apps. Your Slack workspace may require an admin to generate it.
        </p>
      </div>
    </div>
  );
}

function StepNumber({ n }: { n: number }) {
  return (
    <span className="flex size-6 shrink-0 items-center justify-center border text-xs font-medium">
      {n}
    </span>
  );
}
