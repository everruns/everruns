"use client";

import { useUiTranslations, useLocale } from "@/providers/locale-provider";
import { useChatPageRows } from "@/components/chat-mode/page-view-state";

import { useEffect, useState } from "react";
import { useSearchParams } from "next/navigation";
import { usePageTitle } from "@/hooks";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Notice, NoticeDescription } from "@/components/ui/notice";
import { Skeleton } from "@/components/ui/skeleton";
import { SlackIcon as Slack } from "@/components/icons/slack-icon";
import {
  ConnectSlackWorkspaceChoice,
  slackConnectFailureMessage,
  slackWorkspaceLabel,
} from "@/components/slack/slack-workspaces";
import {
  useInvalidateSlackWorkspaces,
  useSlackInstallCapability,
  useSlackWorkspaces,
} from "@/hooks/use-agent-channels";
import {
  disconnectSlackWorkspace,
  testSlackWorkspace,
  type SlackWorkspace,
} from "@/lib/api/agent-channels";

export default function SlackWorkspacesPage() {
  const ui = useUiTranslations();
  usePageTitle(ui("Slack workspaces"), ui("Settings"));
  const capability = useSlackInstallCapability();
  const supported = capability.data?.supported === true;
  const canManage = capability.data?.can_manage === true;
  const managerApp = capability.data?.manager_app === true;
  const workspaces = useSlackWorkspaces(supported);
  const invalidate = useInvalidateSlackWorkspaces();
  const [connecting, setConnecting] = useState(false);
  // Slack's consent returns here with `slack_connect=ok|failed` (and Slack's code on failure).
  // Captured once on mount, then dropped from the address bar so a reload does not repeat it.
  const searchParams = useSearchParams();
  const [consentResult] = useState(() => {
    const outcome = searchParams.get("slack_connect");
    if (!outcome) return null;
    return outcome === "ok"
      ? { ok: true, text: "Slack workspace connected." }
      : { ok: false, text: slackConnectFailureMessage(searchParams.get("slack_error")) };
  });
  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    if (!params.has("slack_connect")) return;
    params.delete("slack_connect");
    params.delete("slack_error");
    const query = params.toString();
    window.history.replaceState(null, "", `${window.location.pathname}${query ? `?${query}` : ""}`);
  }, []);

  useChatPageRows(
    "Slack workspaces",
    (workspaces.data ?? []).map((w) => ({
      id: w.id,
      status: w.status,
      connected_at: w.connected_at,
    })),
    { enabled: supported, loading: workspaces.isLoading, error: !!workspaces.error },
  );

  if (capability.isLoading || (supported && workspaces.isLoading)) {
    return <Skeleton className="h-40 w-full" />;
  }

  const list = workspaces.data ?? [];
  const showWizard = canManage && (connecting || list.length === 0);

  return (
    <div className="space-y-6">
      <div className="space-y-1">
        <h2 className="text-lg font-semibold">{ui("Slack workspaces")}</h2>
        <p className="text-sm text-muted-foreground">
          {ui(
            "Connect a workspace once, and every agent you put in Slack gets its own Slack app there, its own name in the Agents menu, with one click and one Slack approval.",
          )}
        </p>
      </div>

      {consentResult && (
        <Notice variant={consentResult.ok ? "success" : "destructive"}>
          <NoticeDescription>{consentResult.text}</NoticeDescription>
        </Notice>
      )}

      {!supported && (
        <Notice variant="info">
          <NoticeDescription>
            {ui(
              "This deployment does not create Slack apps for you. Configure each Slack endpoint manually from its settings.",
            )}
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
              managerApp={managerApp}
              onChanged={invalidate}
            />
          ))}
        </Card>
      )}

      {supported && !canManage && list.length === 0 && (
        <Notice variant="info">
          <NoticeDescription>
            {ui(
              "No Slack workspace is connected yet. Ask an organization administrator to connect one here.",
            )}
          </NoticeDescription>
        </Notice>
      )}

      {supported && showWizard && (
        <Card className="space-y-4 p-4">
          <div className="space-y-1">
            <p className="text-sm font-medium">{ui("Connect a Slack workspace")}</p>
            <p className="text-xs text-muted-foreground">
              {managerApp ? ui("One Slack approval") : ui("About thirty seconds")}
              {ui(", once per workspace.")}
            </p>
          </div>
          <ConnectSlackWorkspaceChoice
            managerApp={managerApp}
            onConnected={async () => {
              setConnecting(false);
              await invalidate();
            }}
          />
          {list.length > 0 && (
            <Button type="button" variant="ghost" size="sm" onClick={() => setConnecting(false)}>
              {ui("Cancel")}
            </Button>
          )}
        </Card>
      )}

      {supported && canManage && list.length > 0 && !connecting && (
        <Button type="button" variant="outline" onClick={() => setConnecting(true)}>
          <Slack className="size-4" />
          {ui("Connect another workspace")}
        </Button>
      )}
    </div>
  );
}

function WorkspaceRow({
  workspace,
  canManage,
  managerApp,
  onChanged,
}: {
  workspace: SlackWorkspace;
  canManage: boolean;
  managerApp: boolean;
  onChanged: () => Promise<unknown>;
}) {
  const { displayLocale } = useLocale();

  const ui = useUiTranslations();
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
              {ui("Connected")} {new Date(workspace.connected_at).toLocaleDateString(displayLocale)}
              {workspace.connected_with === "manager_app" && " with Slack approval"}
            </p>
          </div>
          {needsReconnect ? (
            <Badge variant="destructive">{ui("Reconnect required")}</Badge>
          ) : (
            <Badge variant="success">{ui("Connected")}</Badge>
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
                {ui("Reconnect")}
              </Button>
            ) : (
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => void run("test")}
                disabled={pending !== null}
              >
                {pending === "test" ? ui("Testing…") : ui("Test")}
              </Button>
            )}
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => setConfirming(true)}
              disabled={pending === "test" || pending === "disconnect"}
            >
              {ui("Disconnect")}
            </Button>
          </div>
        )}
      </div>
      {needsReconnect && (
        <p className="text-xs text-muted-foreground">
          {ui(
            "Slack stopped accepting the saved credential. Agents already in this workspace keep working; new agents cannot be added until it is reconnected.",
          )}
        </p>
      )}
      {confirming && (
        <Notice variant="warning">
          <NoticeDescription>
            <span className="block">
              {ui(
                "Agents already in this workspace keep working; each has its own Slack app. You will not be able to add agents to it, or update their Slack apps, until you connect it again.",
              )}
            </span>
            <span className="mt-3 flex gap-2">
              <Button
                type="button"
                size="sm"
                variant="destructive"
                onClick={() => void run("disconnect")}
                disabled={pending !== null}
              >
                {pending === "disconnect" ? ui("Disconnecting…") : ui("Disconnect workspace")}
              </Button>
              <Button type="button" size="sm" variant="ghost" onClick={() => setConfirming(false)}>
                {ui("Keep it")}
              </Button>
            </span>
          </NoticeDescription>
        </Notice>
      )}
      {pending === "reconnect" && (
        <ConnectSlackWorkspaceChoice
          managerApp={managerApp}
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
