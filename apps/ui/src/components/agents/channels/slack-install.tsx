"use client";

// One-click Slack install for a channel: the install hook and the workspace
// picker. Split out of channel-form.tsx, which is near the file-size limit.

import { useCallback, useEffect, useState } from "react";
import Link from "next/link";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { beginSlackInstall } from "@/lib/api/agent-channels";
import type { SlackInstallCapability } from "@/lib/api/agent-channels";
import { ApiError } from "@/lib/api/client";
import { useInvalidateSlackWorkspaces, useSlackWorkspaces } from "@/hooks/use-agent-channels";
import { ConnectSlackWorkspace, slackWorkspaceLabel } from "@/components/slack/slack-workspaces";

/**
 * Drives the one-click Slack install for an existing channel (EVE-1069).
 *
 * `unavailable` is not an error state. A deployment holding no Slack app
 * configuration token answers 501, which is the self-hosted steady state: the
 * manual fields are that deployment's supported path, not a fallback from a
 * failure, so the UI opens them rather than reporting something went wrong.
 */
export function useSlackInstall(channelId?: string, teamId?: string, onUnavailable?: () => void) {
  const [pending, setPending] = useState(false);
  const [unavailable, setUnavailable] = useState(false);
  const [error, setError] = useState<Error | null>(null);

  const begin = useCallback(async () => {
    if (!channelId) return;
    setPending(true);
    setError(null);
    try {
      const { authorize_url } = await beginSlackInstall(channelId, teamId || null);
      // A full navigation, not a router push: the next hop is Slack's consent
      // screen, which is outside this app.
      window.location.href = authorize_url;
    } catch (caught) {
      if (caught instanceof ApiError && caught.status === 501) {
        setUnavailable(true);
        onUnavailable?.();
      } else {
        setError(
          caught instanceof Error ? caught : new Error("Could not start the Slack install."),
        );
      }
      setPending(false);
    }
  }, [channelId, teamId, onUnavailable]);

  return { begin, pending, unavailable, error } as const;
}

/**
 * Where the agent's Slack app will be created (EVE-1148).
 *
 * Workspaces are connected once, by an admin, in Settings → Slack workspaces. Here a builder only
 * picks one; with a single workspace there is nothing to pick and it is simply shown. An admin
 * with none connected can connect one inline without leaving the form.
 */
export function SlackWorkspaceChoice({
  capability,
  selected,
  onSelect,
  onChanged,
}: {
  capability: SlackInstallCapability;
  selected: string;
  onSelect: (teamId: string) => void;
  onChanged?: () => void | Promise<unknown>;
}) {
  const workspaces = useSlackWorkspaces(capability.supported);
  const invalidateWorkspaces = useInvalidateSlackWorkspaces();
  const connected = async () => {
    await invalidateWorkspaces();
    await onChanged?.();
  };
  const usable = (workspaces.data ?? []).filter(
    (workspace) => workspace.status === "connected" && workspace.team_id,
  );
  const stale = (workspaces.data ?? []).filter(
    (workspace) => workspace.status === "reconnect_required",
  );
  const onlyTeam = usable.length === 1 ? usable[0].team_id : null;

  // One workspace: select it so the consent screen opens on it, without asking.
  useEffect(() => {
    if (onlyTeam && selected !== onlyTeam) onSelect(onlyTeam);
  }, [onlyTeam, selected, onSelect]);

  if (workspaces.isLoading) {
    return <p className="text-xs text-muted-foreground">Loading Slack workspaces…</p>;
  }

  if (usable.length === 0) {
    return (
      <div className="space-y-4 border p-4">
        <div className="space-y-1">
          <p className="text-sm font-medium">
            {stale.length > 0 ? "Reconnect your Slack workspace" : "Connect a Slack workspace"}
          </p>
          <p className="text-xs text-muted-foreground">
            {stale.length > 0
              ? "Slack stopped accepting the saved token. Reconnect once for the organization, then add agents with one click."
              : "Once per organization. Afterwards, each agent gets its own Slack app with one click."}
          </p>
        </div>
        {capability.can_manage ? (
          <ConnectSlackWorkspace reconnect={stale.length > 0} onConnected={connected} />
        ) : (
          <p className="text-xs text-muted-foreground">
            Ask an organization administrator to connect a workspace in{" "}
            <Link className="underline" href="/settings/slack">
              Settings → Slack workspaces
            </Link>
            . You can still configure this channel manually below.
          </p>
        )}
      </div>
    );
  }

  return (
    <div className="space-y-2">
      {usable.length === 1 ? (
        <p className="text-sm">
          Slack workspace: <strong>{slackWorkspaceLabel(usable[0])}</strong>
        </p>
      ) : (
        <div className="space-y-2">
          <Label htmlFor="slack_install_workspace">Slack workspace</Label>
          <Select value={selected} onValueChange={(value) => onSelect(String(value ?? ""))}>
            <SelectTrigger id="slack_install_workspace" className="w-full sm:w-80">
              <SelectValue placeholder="Choose a workspace" />
            </SelectTrigger>
            <SelectContent>
              {usable.map((workspace) => (
                <SelectItem key={workspace.id} value={workspace.team_id ?? ""}>
                  {slackWorkspaceLabel(workspace)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      )}
      <p className="text-xs text-muted-foreground">
        If this workspace requires admins to approve apps, Slack sends an approval request instead
        of installing straight away.{" "}
        <Link className="underline" href="/settings/slack">
          Manage workspaces
        </Link>
      </p>
    </div>
  );
}
