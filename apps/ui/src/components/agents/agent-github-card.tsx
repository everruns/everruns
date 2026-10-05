"use client";

import { useState } from "react";
import Link from "next/link";
import { ExternalLink, GitPullRequest, Plus } from "lucide-react";
import {
  useAgentGitHub,
  useConnectAgentGitHub,
  useDisconnectAgentGitHub,
} from "@/hooks/use-agent-github";
import { Button, buttonVariants } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

export function AgentGitHubCard({ agentId }: { agentId: string }) {
  const { data: status, isLoading } = useAgentGitHub(agentId);
  const connect = useConnectAgentGitHub(agentId);
  const disconnect = useDisconnectAgentGitHub(agentId);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const onConnect = async () => {
    setError(null);
    try {
      await connect.mutateAsync();
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Unable to start GitHub connection.");
    }
  };

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

  return (
    <Card>
      <CardContent className="space-y-3">
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div className="flex min-w-0 flex-[1_1_24rem] items-start gap-3">
            <span className="flex size-9 shrink-0 items-center justify-center border bg-muted">
              <GitPullRequest className="size-4" aria-hidden="true" />
            </span>
            <div className="min-w-0 space-y-1">
              <h4 className="font-medium">GitHub</h4>
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
                </p>
              )}
            </div>
          </div>
          {status?.connected ? (
            <div className="flex flex-wrap items-center gap-2">
              <Link
                href={`/agents/${agentId}/triggers/new?type=github`}
                className={buttonVariants({ size: "sm" })}
              >
                <Plus className="size-4" /> Add pull request trigger
              </Link>
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
            <Button variant="outline" size="sm" onClick={onConnect} disabled={connect.isPending}>
              {status.app_created ? "Finish installing" : "Connect GitHub"}
            </Button>
          ) : null}
        </div>
        {error && <p className="text-sm text-destructive">{error}</p>}
      </CardContent>

      <Dialog open={confirmOpen} onOpenChange={setConfirmOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Disconnect GitHub?</DialogTitle>
            <DialogDescription>
              This uninstalls the agent&apos;s GitHub App. GitHub triggers will stop firing.
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
