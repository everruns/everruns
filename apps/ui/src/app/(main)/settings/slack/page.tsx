"use client";

import { useState } from "react";
import { usePageTitle } from "@/hooks";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Notice, NoticeDescription } from "@/components/ui/notice";
import { Skeleton } from "@/components/ui/skeleton";
import { SlackIcon as Slack } from "@/components/icons/slack-icon";
import { ConnectSlackWorkspace, slackWorkspaceLabel } from "@/components/slack/slack-workspaces";
import {
  useInvalidateSlackWorkspaces,
  useSlackInstallCapability,
  useSlackWorkspaces,
} from "@/hooks/use-agent-endpoints";
import {
  disconnectSlackWorkspace,
  testSlackWorkspace,
  type SlackWorkspace,
} from "@/lib/api/agent-endpoints";

export default function SlackWorkspacesPage() {
  usePageTitle("Slack workspaces", "Settings");
  const capability = useSlackInstallCapability();
  const supported = capability.data?.supported === true;
  const canManage = capability.data?.can_manage === true;
  const workspaces = useSlackWorkspaces(supported);
  const invalidate = useInvalidateSlackWorkspaces();
  const [connecting, setConnecting] = useState(false);

  if (capability.isLoading || (supported && workspaces.isLoading)) {
    return <Skeleton className="h-40 w-full" />;
  }

  const list = workspaces.data ?? [];
  const showWizard = canManage && (connecting || list.length === 0);

  return (
    <div className="space-y-6">
      <div className="space-y-1">
        <h2 className="text-lg font-semibold">Slack workspaces</h2>
        <p className="text-sm text-muted-foreground">
          Connect a workspace once, and every agent you put in Slack gets its own Slack app there —
          its own name in the Agents menu — with one click and one Slack approval.
        </p>
      </div>

      {!supported && (
        <Notice variant="info">
          <NoticeDescription>
            This deployment does not create Slack apps for you. Configure each Slack endpoint
            manually from its settings.
          </NoticeDescription>
        </Notice>
      )}

      {supported && list.length > 0 && (
        <Card className="divide-y p-0">
          {list.map((workspace) => (
            <WorkspaceRow
              key={workspace.id}
              workspace={workspace}
              canManage={canManage}
              onChanged={invalidate}
            />
          ))}
        </Card>
      )}

      {supported && !canManage && list.length === 0 && (
        <Notice variant="info">
          <NoticeDescription>
            No Slack workspace is connected yet. Ask an organization administrator to connect one
            here.
          </NoticeDescription>
        </Notice>
      )}

      {supported && showWizard && (
        <Card className="space-y-4 p-4">
          <div className="space-y-1">
            <p className="text-sm font-medium">Connect a Slack workspace</p>
            <p className="text-xs text-muted-foreground">
              About thirty seconds, once per workspace.
            </p>
          </div>
          <ConnectSlackWorkspace
            onConnected={async () => {
              setConnecting(false);
              await invalidate();
            }}
          />
          {list.length > 0 && (
            <Button type="button" variant="ghost" size="sm" onClick={() => setConnecting(false)}>
              Cancel
            </Button>
          )}
        </Card>
      )}

      {supported && canManage && list.length > 0 && !connecting && (
        <Button type="button" variant="outline" onClick={() => setConnecting(true)}>
          <Slack className="size-4" />
          Connect another workspace
        </Button>
      )}
    </div>
  );
}

function WorkspaceRow({
  workspace,
  canManage,
  onChanged,
}: {
  workspace: SlackWorkspace;
  canManage: boolean;
  onChanged: () => Promise<unknown>;
}) {
  const [pending, setPending] = useState<"test" | "disconnect" | "reconnect" | null>(null);
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);
  const [confirming, setConfirming] = useState(false);
  const needsReconnect = workspace.status === "reconnect_required";

  const run = async (action: "test" | "disconnect") => {
    setPending(action);
    setMessage(null);
    try {
      if (action === "test") {
        await testSlackWorkspace(workspace.id);
        setMessage({ ok: true, text: "Connection works." });
      } else {
        await disconnectSlackWorkspace(workspace.id);
        await onChanged();
      }
    } catch (caught) {
      setMessage({
        ok: false,
        text: caught instanceof Error ? caught.message : "Slack did not accept the request.",
      });
    } finally {
      setPending(null);
      setConfirming(false);
    }
  };

  return (
    <div className="space-y-3 p-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-3">
          <Slack className="size-5" />
          <div>
            <p className="text-sm font-medium">{slackWorkspaceLabel(workspace)}</p>
            <p className="text-xs text-muted-foreground">
              Connected {new Date(workspace.connected_at).toLocaleDateString()}
            </p>
          </div>
          {needsReconnect ? (
            <Badge variant="destructive">Reconnect required</Badge>
          ) : (
            <Badge variant="success">Connected</Badge>
          )}
        </div>
        {canManage && (
          <div className="flex gap-2">
            {needsReconnect ? (
              <Button
                type="button"
                size="sm"
                onClick={() => setPending(pending === "reconnect" ? null : "reconnect")}
              >
                Reconnect
              </Button>
            ) : (
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => void run("test")}
                disabled={pending !== null}
              >
                {pending === "test" ? "Testing…" : "Test"}
              </Button>
            )}
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => setConfirming(true)}
              disabled={pending === "test" || pending === "disconnect"}
            >
              Disconnect
            </Button>
          </div>
        )}
      </div>
      {needsReconnect && (
        <p className="text-xs text-muted-foreground">
          Slack stopped accepting the saved token. Agents already in this workspace keep working;
          new agents cannot be added until it is reconnected.
        </p>
      )}
      {confirming && (
        <Notice variant="warning">
          <NoticeDescription>
            <span className="block">
              Agents already in this workspace keep working — each has its own Slack app. You will
              not be able to add agents to it, or update their Slack apps, until you connect it
              again.
            </span>
            <span className="mt-3 flex gap-2">
              <Button
                type="button"
                size="sm"
                variant="destructive"
                onClick={() => void run("disconnect")}
                disabled={pending !== null}
              >
                {pending === "disconnect" ? "Disconnecting…" : "Disconnect workspace"}
              </Button>
              <Button type="button" size="sm" variant="ghost" onClick={() => setConfirming(false)}>
                Keep it
              </Button>
            </span>
          </NoticeDescription>
        </Notice>
      )}
      {pending === "reconnect" && (
        <ConnectSlackWorkspace
          reconnect
          onConnected={async () => {
            setPending(null);
            await onChanged();
          }}
        />
      )}
      {message && (
        <p className={message.ok ? "text-xs text-success" : "text-xs text-destructive"}>
          {message.text}
        </p>
      )}
    </div>
  );
}
