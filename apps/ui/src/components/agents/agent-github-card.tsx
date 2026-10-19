"use client";

// The agent page's GitHub section: who the agent is on GitHub, where that
// identity is installed, and what draws on it.
//
// Decision (D7, knowledge/integrations/github-model.md): GitHub is an identity,
// not a trigger, so it is a section of its own between Channels and Triggers.
// Triggers link here for setup instead of hosting it.
//
// Decision (D3): an agent without a GitHub identity chooses one explicitly:
// its own App (recommended), or the shared Everruns App through an account an
// org admin linked in Settings > Organization. The shared choice is offered
// only when the org has a link; nothing falls back to it silently.
//
// Decision (D2): GitHub has no API to set an App logo, so a connected agent App
// shows a "Set the avatar" step: download the agent's avatar, open the App's
// settings page. Whether a logo is already set is not detected.

import { useState } from "react";
import Link from "next/link";
import {
  ChevronDown,
  Download,
  ExternalLink,
  GitPullRequest,
  MoreHorizontal,
  Plus,
} from "lucide-react";
import {
  useAgentGitHub,
  useChooseAgentEverrunsApp,
  useDisconnectAgentGitHub,
} from "@/hooks/use-agent-github";
import { ConnectGitHubButton } from "@/components/agents/connect-github-button";
import { Badge } from "@/components/ui/badge";
import { Button, buttonVariants } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuPositioner,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { agentAvatarUrl } from "@/lib/api/agents";
import type { AgentAvatar } from "@/lib/api/agent-types";
import type { AgentGitHubStatus } from "@/lib/api/agent-github";
import { pluralize } from "@/lib/formatting";

const KIND_LABELS: Record<NonNullable<AgentGitHubStatus["kind"]>, string> = {
  agent_app: "Own GitHub App",
  everruns_app: "Everruns App",
};

/** The GitHub section of the agent page, a sibling of Channels and Triggers. */
export function AgentGitHubSection({
  agentId,
  avatar,
  githubTriggerCount,
}: {
  agentId: string;
  avatar?: AgentAvatar | null;
  githubTriggerCount?: number;
}) {
  return (
    <section id="github" className="flex min-w-0 flex-col gap-3">
      <div>
        <h3 className="text-lg font-semibold tracking-tight">GitHub</h3>
        <p className="text-sm text-muted-foreground">Who this agent is on GitHub</p>
      </div>
      <AgentGitHubCard agentId={agentId} avatar={avatar} githubTriggerCount={githubTriggerCount} />
    </section>
  );
}

function AvatarStep({
  avatar,
  settingsUrl,
}: {
  avatar?: AgentAvatar | null;
  settingsUrl: string | null;
}) {
  const largest = avatar ? Math.max(...avatar.sizes) : null;
  return (
    <div className="space-y-2 border-t pt-3">
      <h5 className="text-sm font-medium">Set the avatar</h5>
      <p className="text-sm text-muted-foreground">
        GitHub shows this agent with a default logo until one is uploaded to its App, and GitHub
        offers no way to set it for you.{" "}
        {avatar
          ? "Download the agent's avatar, then upload it as the logo in the App's settings."
          : "Upload a logo in the App's settings, or give the agent an avatar first."}
      </p>
      <div className="flex flex-wrap items-center gap-2">
        {avatar && largest !== null && (
          <a
            href={agentAvatarUrl(avatar, largest)}
            download="avatar.png"
            className={buttonVariants({ variant: "outline", size: "sm" })}
          >
            <Download className="size-4" /> Download avatar
          </a>
        )}
        {settingsUrl && (
          <a
            href={settingsUrl}
            target="_blank"
            rel="noopener noreferrer"
            className={buttonVariants({ variant: "outline", size: "sm" })}
          >
            Open App settings <ExternalLink className="size-4" />
          </a>
        )}
      </div>
    </div>
  );
}

export function AgentGitHubCard({
  agentId,
  avatar,
  githubTriggerCount,
}: {
  agentId: string;
  avatar?: AgentAvatar | null;
  /** GitHub triggers on this agent, when the caller already has them. */
  githubTriggerCount?: number;
}) {
  const { data: status, isLoading } = useAgentGitHub(agentId);
  const disconnect = useDisconnectAgentGitHub(agentId);
  const choose = useChooseAgentEverrunsApp(agentId);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const onDisconnect = async () => {
    if (!status?.identity_id) return;
    setError(null);
    try {
      await disconnect.mutateAsync(status.identity_id);
      setConfirmOpen(false);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Unable to disconnect GitHub.");
    }
  };

  const onChoose = async (connectionId: string) => {
    setError(null);
    try {
      await choose.mutateAsync(connectionId);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Unable to use the Everruns app.");
    }
  };

  const kindLabel = status?.kind ? KIND_LABELS[status.kind] : null;
  const shared = status?.kind === "everruns_app";
  // The choice is offered only before the agent has an App of its own.
  const choices = status && !status.app_created ? status.everruns_app_installations : [];

  return (
    <Card>
      <CardContent className="space-y-3">
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div className="flex min-w-0 flex-[1_1_24rem] items-start gap-3">
            <span className="flex size-9 shrink-0 items-center justify-center border bg-muted">
              <GitPullRequest className="size-4" aria-hidden="true" />
            </span>
            <div className="min-w-0 space-y-1">
              <h4 className="break-words font-medium">{status?.bot_login ?? "GitHub"}</h4>
              {kindLabel && <p className="text-sm text-muted-foreground">{kindLabel}</p>}
              {isLoading || !status ? (
                <p className="text-sm text-muted-foreground">Loading GitHub status…</p>
              ) : status.connected ? (
                <>
                  <p className="break-words text-sm">{status.account ?? "GitHub App installed"}</p>
                  <p className="text-sm text-muted-foreground">
                    {status.repository_selection === "selected"
                      ? "Selected repositories"
                      : "All repositories"}
                  </p>
                </>
              ) : (
                <p className="text-sm text-muted-foreground">
                  Give this agent its own GitHub App to read pull requests and comment as itself.
                  {!status.app_created && " The App can only be installed where it is created."}
                  {choices.length > 0 &&
                    " Or act as the shared Everruns App on an account your organization linked."}
                </p>
              )}
            </div>
          </div>
          {status?.connected ? (
            <div className="flex flex-wrap items-center gap-2">
              {status.app_url && (
                <a
                  href={status.app_url}
                  target="_blank"
                  rel="noopener noreferrer"
                  className={buttonVariants({ variant: "outline", size: "sm" })}
                >
                  Manage on GitHub <ExternalLink className="size-4" />
                </a>
              )}
              <Button variant="outline" size="sm" onClick={() => setConfirmOpen(true)}>
                Disconnect
              </Button>
            </div>
          ) : !isLoading && status ? (
            <div className="flex flex-wrap items-center gap-2">
              <ConnectGitHubButton
                agentId={agentId}
                status={status}
                label={status.app_created ? undefined : "Create this agent's own GitHub App"}
              />
              {!status.app_created && <Badge variant="outline">Recommended</Badge>}
              {choices.length > 0 && (
                <DropdownMenu>
                  <DropdownMenuTrigger
                    className={buttonVariants({ variant: "ghost", size: "sm" })}
                    disabled={choose.isPending}
                  >
                    Use the Everruns app <ChevronDown className="size-4" aria-hidden="true" />
                  </DropdownMenuTrigger>
                  <DropdownMenuPositioner align="start">
                    <DropdownMenuContent className="w-60">
                      <DropdownMenuGroup>
                        <DropdownMenuLabel>Linked GitHub accounts</DropdownMenuLabel>
                        {choices.map((choice) => (
                          <DropdownMenuItem
                            key={choice.connection_id}
                            onClick={() => onChoose(choice.connection_id)}
                          >
                            <span className="truncate">{choice.account}</span>
                          </DropdownMenuItem>
                        ))}
                      </DropdownMenuGroup>
                    </DropdownMenuContent>
                  </DropdownMenuPositioner>
                </DropdownMenu>
              )}
              {status.app_created && (
                // An App that was never installed can be forgotten, e.g. to
                // create it in an organization instead of a personal account.
                <DropdownMenu>
                  <DropdownMenuTrigger
                    className={buttonVariants({ variant: "ghost", size: "icon" })}
                    aria-label="GitHub actions"
                  >
                    <MoreHorizontal className="size-4" />
                  </DropdownMenuTrigger>
                  <DropdownMenuPositioner>
                    <DropdownMenuContent>
                      <DropdownMenuItem onClick={onDisconnect} disabled={disconnect.isPending}>
                        Start over
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenuPositioner>
                </DropdownMenu>
              )}
            </div>
          ) : null}
        </div>
        {status?.connected && status.missing_permissions.length > 0 && (
          <p className="border bg-muted/50 p-3 text-sm" role="status">
            This agent&apos;s GitHub App was created before CI triggers and lacks{" "}
            {status.missing_permissions.join(", ")}. Open the App settings on GitHub, grant those
            permissions, subscribe to the Workflow run and Check suite events, then accept the
            change on the installation.
          </p>
        )}
        {status?.connected && status.kind === "agent_app" && (
          <AvatarStep avatar={avatar} settingsUrl={status.app_settings_url ?? status.app_url} />
        )}
        {status?.connected && (
          <div className="flex flex-wrap items-center gap-2 border-t pt-3">
            <Link
              href={`/agents/${agentId}/triggers/new?type=github`}
              className={buttonVariants({ size: "sm" })}
            >
              <Plus className="size-4" /> Add pull request trigger
            </Link>
            <Link
              href={`/agents/${agentId}?tab=integrations#triggers`}
              className={buttonVariants({ variant: "ghost", size: "sm" })}
            >
              {githubTriggerCount === undefined
                ? "GitHub triggers"
                : `${githubTriggerCount} GitHub ${pluralize(githubTriggerCount, "trigger")}`}
            </Link>
          </div>
        )}
        {error && <p className="text-sm text-destructive">{error}</p>}
      </CardContent>

      <Dialog open={confirmOpen} onOpenChange={setConfirmOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Disconnect GitHub?</DialogTitle>
            <DialogDescription>
              {shared
                ? "The agent stops acting as the Everruns App. The installation stays linked to the organization. GitHub triggers will stop firing."
                : "This uninstalls the agent's GitHub App. GitHub triggers will stop firing."}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setConfirmOpen(false)}>
              Cancel
            </Button>
            <Button variant="destructive" onClick={onDisconnect} disabled={disconnect.isPending}>
              Disconnect
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </Card>
  );
}
