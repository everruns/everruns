"use client";

import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { ExternalLink, RefreshCw } from "lucide-react";
import { useAgentSource, agentSourceKey } from "@/hooks/use-agent-source";
import { useAgentGitHub, useGitHubRepositories } from "@/hooks/use-agent-github";
import { usePolicies } from "@/hooks/use-policies";
import { queryKeys } from "@/lib/query-keys";
import {
  connectAgentSource,
  disconnectAgentSource,
  getAgentSourceOptions,
  previewAgentSource,
  syncAgentSource,
  type SourceLocation,
  type SourcePreview,
  type SourceOptions,
} from "@/lib/api/agent-source";
import { AgentPackageReview } from "./agent-package-review";
import { Button, buttonVariants } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";

function date(value: string | null) {
  return value ? new Date(value).toLocaleString() : "Never";
}
export function AgentSourcePanel({
  agentId,
  disabled = false,
}: {
  agentId: string;
  disabled?: boolean;
}) {
  const { data: source, isLoading, error: loadError } = useAgentSource(agentId);
  const { data: github } = useAgentGitHub(agentId);
  const { data: repositories, error: repoError } = useGitHubRepositories(
    github?.identity_id,
    !!github?.connected,
  );
  const queryClient = useQueryClient();
  const { can } = usePolicies("agents");
  const canManage = can("agent.manage") && !disabled;
  const [open, setOpen] = useState(false);
  const [disconnectOpen, setDisconnectOpen] = useState(false);
  const [location, setLocation] = useState<SourceLocation>({
    repository: "",
    branch: "",
    path: "",
  });
  const [options, setOptions] = useState<SourceOptions | null>(null);
  const [preview, setPreview] = useState<SourcePreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  async function perform(action: () => Promise<void>) {
    setBusy(true);
    setError(null);
    try {
      await action();
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Could not update Agent source.");
    } finally {
      setBusy(false);
    }
  }
  async function refresh() {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: agentSourceKey(agentId) }),
      queryClient.invalidateQueries({ queryKey: queryKeys.agents.all }),
      queryClient.invalidateQueries({ queryKey: queryKeys.agents.detail(agentId) }),
      queryClient.invalidateQueries({ queryKey: queryKeys.agentChannels.all(agentId) }),
    ]);
  }
  function change(next: SourceLocation) {
    setLocation(next);
    setPreview(null);
  }
  function start() {
    setLocation(
      source
        ? { repository: source.repository, branch: source.branch, path: source.path }
        : { repository: "", branch: "", path: "" },
    );
    setPreview(null);
    setOptions(null);
    setError(null);
    setOpen(true);
  }
  async function browse(next: SourceLocation) {
    change(next);
    const result = await getAgentSourceOptions(agentId, {
      ...next,
      branch: next.branch || undefined,
    });
    setOptions(result);
    setLocation({ ...next, branch: result.branch });
  }
  const sourceUrl = source
    ? `https://github.com/${source.repository}/tree/${source.applied_commit || encodeURIComponent(source.branch)}${source.path ? `/${source.path.split("/").map(encodeURIComponent).join("/")}` : ""}`
    : null;
  return (
    <section className="space-y-3 border bg-card p-4" aria-label="Agent source" id="source">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0 space-y-1">
          <h2 className="font-semibold">Source</h2>
          {isLoading ? (
            <p className="text-sm text-muted-foreground">Loading source…</p>
          ) : loadError ? (
            <p role="alert" className="break-words text-sm text-destructive">
              Could not load Agent source.
            </p>
          ) : source ? (
            <>
              <p className="break-all text-sm">
                {source.repository} · {source.branch} · {source.path || "/"}
              </p>
              <p className="text-sm text-muted-foreground">
                Managed by GitHub. Edit the repository to update this Agent.
              </p>
              <div className="flex flex-wrap gap-2 text-xs text-muted-foreground">
                <Badge variant="outline">
                  {source.status === "synced"
                    ? "Synced"
                    : source.status === "error"
                      ? "Sync failed"
                      : source.status === "syncing"
                        ? "Syncing"
                        : "Queued"}
                </Badge>
                {source.applied_commit && (
                  <span className="font-mono">{source.applied_commit.slice(0, 7)}</span>
                )}
                <span>Last checked: {date(source.last_checked_at)}</span>
                <span>Last synced: {date(source.last_synced_at)}</span>
              </div>
            </>
          ) : (
            <p className="text-sm text-muted-foreground">
              Keep this Agent&apos;s instructions, configuration, files, and skills in a GitHub
              repository. Changes sync continuously.
            </p>
          )}
        </div>
        {canManage && !isLoading && !loadError && (
          <div className="flex flex-wrap gap-2">
            {source && (
              <Button
                size="sm"
                variant="outline"
                disabled={busy || source.status === "pending" || source.status === "syncing"}
                onClick={() =>
                  perform(async () => {
                    await syncAgentSource(agentId);
                    await refresh();
                  })
                }
              >
                <RefreshCw className="size-4" /> Sync now
              </Button>
            )}
            <Button size="sm" variant="outline" onClick={start}>
              {source ? "Change source" : "Sync from GitHub"}
            </Button>
            {source && (
              <Button size="sm" variant="ghost" onClick={() => setDisconnectOpen(true)}>
                Disconnect
              </Button>
            )}
          </div>
        )}
      </div>
      {sourceUrl && (
        <a
          href={sourceUrl}
          target="_blank"
          rel="noopener noreferrer"
          className={buttonVariants({ variant: "link", size: "sm" })}
        >
          View on GitHub <ExternalLink className="size-4" />
        </a>
      )}
      {source?.error && (
        <p role="alert" className="break-words text-sm text-destructive">
          {source.error} The last successfully synced definition is retained.
        </p>
      )}
      {error && !open && (
        <p role="alert" className="break-words text-sm text-destructive">
          {error}
        </p>
      )}
      <Dialog
        open={open}
        onOpenChange={(value) => {
          if (!busy) setOpen(value);
        }}
      >
        <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-4xl">
          <DialogHeader>
            <DialogTitle>{source ? "Change Agent source" : "Sync from GitHub"}</DialogTitle>
            <DialogDescription>
              Select a repository, branch, and package folder or manifest. Review changes before
              enabling continuous sync. Synced fields become read-only in Everruns.
            </DialogDescription>
          </DialogHeader>
          {!github?.connected ? (
            <p className="text-sm">
              Connect GitHub in this Agent&apos;s{" "}
              <a
                href={`/agents/${agentId}?tab=integrations#github`}
                className="underline"
                onClick={() => setOpen(false)}
              >
                Integrations
              </a>{" "}
              first.
            </p>
          ) : (
            <>
              <div className="grid gap-4 sm:grid-cols-2">
                <div className="space-y-2">
                  <Label htmlFor="source-repository">Repository</Label>
                  <select
                    id="source-repository"
                    className="h-9 w-full border bg-background px-3 text-sm"
                    value={location.repository}
                    disabled={busy}
                    onChange={(event) => {
                      const repo = event.target.value;
                      setOptions(null);
                      perform(() => browse({ repository: repo, branch: "", path: "" }));
                    }}
                  >
                    <option value="">Select a repository</option>
                    {repositories?.map((repo) => (
                      <option key={repo.full_name} value={repo.full_name}>
                        {repo.full_name}
                      </option>
                    ))}
                  </select>
                  {repoError && (
                    <p role="alert" className="break-words text-sm text-destructive">
                      Could not list repositories. Check the GitHub connection.
                    </p>
                  )}
                </div>
                <div className="space-y-2">
                  <Label htmlFor="source-branch">Branch</Label>
                  <Input
                    id="source-branch"
                    list="source-branches"
                    value={location.branch}
                    disabled={busy || !location.repository}
                    onChange={(event) => {
                      change({ ...location, branch: event.target.value, path: "" });
                      setOptions(null);
                    }}
                    placeholder="main"
                  />
                  <datalist id="source-branches">
                    {options?.branches.map((branch) => (
                      <option key={branch} value={branch}>
                        {branch}
                      </option>
                    ))}
                  </datalist>
                </div>
                <div className="space-y-2 sm:col-span-2">
                  <Label htmlFor="source-path">Package folder or manifest</Label>
                  <div className="flex gap-2">
                    <Input
                      id="source-path"
                      value={location.path}
                      disabled={busy || !location.repository}
                      onChange={(event) => change({ ...location, path: event.target.value })}
                      placeholder="agents/support or agents/support/agent.toml"
                    />
                    <Button
                      variant="outline"
                      disabled={busy || !location.repository || !location.branch}
                      onClick={() => perform(() => browse(location))}
                    >
                      Browse folder
                    </Button>
                  </div>
                  <p className="text-xs text-muted-foreground">
                    Leave empty for the repository root. Folder packages include declared files and
                    skills.
                  </p>
                  {options && (
                    <div className="flex flex-wrap gap-2">
                      {location.path && (
                        <Button
                          variant="ghost"
                          size="sm"
                          disabled={busy}
                          onClick={() =>
                            perform(() =>
                              browse({
                                ...location,
                                path: location.path
                                  .replace(/\/$/, "")
                                  .split("/")
                                  .slice(0, -1)
                                  .join("/"),
                              }),
                            )
                          }
                        >
                          Parent folder
                        </Button>
                      )}
                      {options.paths.map((path) => (
                        <Button
                          key={path}
                          size="sm"
                          variant="outline"
                          disabled={busy}
                          onClick={() =>
                            path.endsWith("/")
                              ? perform(() => browse({ ...location, path: path.slice(0, -1) }))
                              : change({ ...location, path })
                          }
                        >
                          {path}
                        </Button>
                      ))}
                    </div>
                  )}
                </div>
              </div>
              <Button
                variant="outline"
                disabled={busy || !location.repository || !location.branch}
                onClick={() =>
                  perform(async () => {
                    setPreview(null);
                    setPreview(await previewAgentSource(agentId, location));
                  })
                }
              >
                {busy ? "Loading…" : "Preview changes"}
              </Button>
              {preview?.preview && (
                <AgentPackageReview preview={preview.preview} changes={preview.changes} />
              )}
              {preview?.diagnostics?.map((item, index) => (
                <p key={index} role="alert" className="break-words text-sm text-destructive">
                  {item.path}: {item.message}
                </p>
              ))}
            </>
          )}
          {error && (
            <p role="alert" className="break-words text-sm text-destructive">
              {error}
            </p>
          )}
          <DialogFooter>
            <Button variant="outline" disabled={busy} onClick={() => setOpen(false)}>
              Cancel
            </Button>
            <Button
              disabled={busy || !preview?.valid}
              onClick={() =>
                perform(async () => {
                  if (!preview?.valid) return;
                  await connectAgentSource(agentId, {
                    ...location,
                    commit: preview.commit,
                    agent_digest: preview.agent_digest,
                    ...(source ? { expected_generation: source.generation } : {}),
                  });
                  await refresh();
                  setOpen(false);
                })
              }
            >
              {busy ? "Applying…" : "Enable continuous sync"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <Dialog open={disconnectOpen} onOpenChange={setDisconnectOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Disconnect source sync?</DialogTitle>
            <DialogDescription>
              The last synced definition stays in Everruns and becomes editable. GitHub changes will
              stop applying.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" disabled={busy} onClick={() => setDisconnectOpen(false)}>
              Cancel
            </Button>
            <Button
              disabled={busy}
              onClick={() =>
                perform(async () => {
                  await disconnectAgentSource(agentId);
                  await refresh();
                  setDisconnectOpen(false);
                })
              }
            >
              Disconnect source sync
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
}
