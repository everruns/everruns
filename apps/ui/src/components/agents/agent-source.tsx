"use client";
import {
  Select as SharedSelect,
  SelectTrigger as SharedSelectTrigger,
  SelectContent as SharedSelectContent,
  SelectValue as SharedSelectValue,
  SelectItem as SharedSelectItem,
} from "@/components/ui/select";

import { useUiTranslations } from "@/providers/locale-provider";
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
  type AgentSource,
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
function sourceStatus(source: AgentSource) {
  if (source.status === "conflict") return "Conflict";
  if (source.status === "error") return "Sync failed";
  if (source.status === "syncing") return "Syncing";
  if (source.local_dirty) return "Local changes queued";
  return source.status === "synced" ? "Synced" : "Queued";
}
export function AgentSourcePanel({
  agentId,
  disabled = false,
  open,
  onOpenChange,
}: {
  agentId: string;
  disabled?: boolean;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const ui = useUiTranslations();
  const { data: source, isLoading, error: loadError } = useAgentSource(agentId);
  const { data: github } = useAgentGitHub(agentId);
  const { data: repositories, error: repoError } = useGitHubRepositories(
    github?.identity_id,
    !!github?.connected,
  );
  const queryClient = useQueryClient();
  const { can } = usePolicies("agents");
  const canManage = can("agent.manage") && !disabled;
  function setOpen(value: boolean) {
    if (!value) {
      setMode("pull");
      setLocation({ repository: "", branch: "", path: "" });
      setPreview(null);
      setOptions(null);
      setError(null);
    }
    onOpenChange(value);
  }
  const [mode, setMode] = useState<"pull" | "two_way">("pull");
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
    setMode(source?.mode === "two_way" ? "two_way" : "pull");
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
    <>
      {(source || isLoading || loadError) && (
        <section
          className="space-y-3 border bg-card p-4"
          aria-label={ui("Agent source")}
          id="source"
        >
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="min-w-0 space-y-1">
              <h2 className="font-semibold">{ui("Source")}</h2>
              {isLoading ? (
                <p className="text-sm text-muted-foreground">{ui("Loading source…")}</p>
              ) : loadError ? (
                <p role="alert" className="break-words text-sm text-destructive">
                  {ui("Could not load Agent source.")}
                </p>
              ) : source ? (
                <>
                  <p className="break-all text-sm">
                    {source.repository} · {source.branch} · {source.path || "/"}
                  </p>
                  <p className="text-sm text-muted-foreground">
                    {source.mode === "two_way"
                      ? ui("Two-way sync. Edit here or in GitHub; conflicting changes pause sync.")
                      : ui("Managed by GitHub. Edit the repository to update this Agent.")}
                  </p>
                  <div className="flex flex-wrap gap-2 text-xs text-muted-foreground">
                    <Badge variant="outline">{sourceStatus(source)}</Badge>
                    {source.applied_commit && (
                      <span className="font-mono">{source.applied_commit.slice(0, 7)}</span>
                    )}
                    <span>
                      {ui("Last checked:")} {date(source.last_checked_at)}
                    </span>
                    <span>
                      {ui("Last synced:")} {date(source.last_synced_at)}
                    </span>
                  </div>
                </>
              ) : null}
            </div>
            {canManage && source && !isLoading && !loadError && (
              <div className="flex flex-wrap gap-2">
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
                  <RefreshCw className="size-4" /> {ui("Sync now")}
                </Button>
                <Button size="sm" variant="outline" onClick={start} disabled={busy}>
                  {source.status === "conflict" ? ui("Review GitHub version") : ui("Change source")}
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={busy}
                  onClick={() => setDisconnectOpen(true)}
                >
                  {ui("Disconnect")}
                </Button>
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
              {ui("View on GitHub")} <ExternalLink className="size-4" />
            </a>
          )}
          {source?.error && (
            <p role="alert" className="break-words text-sm text-destructive">
              {source.error}{" "}
              {source.mode === "two_way"
                ? ui("Your local definition is retained.")
                : ui("The last successfully synced definition is retained.")}
            </p>
          )}
          {error && !open && (
            <p role="alert" className="break-words text-sm text-destructive">
              {error}
            </p>
          )}
        </section>
      )}
      <Dialog
        open={open && canManage && !isLoading && !loadError}
        onOpenChange={(value) => {
          if (!busy) setOpen(value);
        }}
      >
        <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-4xl">
          <DialogHeader>
            <DialogTitle>{source ? ui("Change Agent source") : ui("Sync with GitHub")}</DialogTitle>
            <DialogDescription>
              {ui(
                "Select a repository, branch, and package folder or manifest. Review changes before enabling continuous sync. Enabling applies the reviewed GitHub version and replaces local authored changes. Choose whether edits can also be saved from Everruns to GitHub.",
              )}
            </DialogDescription>
          </DialogHeader>
          {!github?.connected ? (
            <p className="text-sm">
              {ui("Connect GitHub in this Agent's")}{" "}
              <a
                href={`/agents/${agentId}?tab=integrations#github`}
                className="underline"
                onClick={() => setOpen(false)}
              >
                {ui("Integrations")}
              </a>{" "}
              {ui("first.")}
            </p>
          ) : (
            <>
              <div className="space-y-2">
                <Label htmlFor="source-mode">{ui("Sync direction")}</Label>
                <SharedSelect
                  value={String(mode)}
                  disabled={busy}
                  onValueChange={(selection) => {
                    setMode(selection as "pull" | "two_way");
                    setPreview(null);
                  }}
                >
                  <SharedSelectTrigger
                    id="source-mode"
                    className="h-9 w-full border bg-background px-3 text-sm"
                  >
                    <SharedSelectValue />
                  </SharedSelectTrigger>
                  <SharedSelectContent>
                    <SharedSelectItem value="pull">
                      {ui("Sync from GitHub · read-only in Everruns")}
                    </SharedSelectItem>
                    <SharedSelectItem value="two_way">
                      {ui("Sync with GitHub · edit on either side")}
                    </SharedSelectItem>
                  </SharedSelectContent>
                </SharedSelect>
                {mode === "two_way" && (
                  <p className="text-sm text-muted-foreground">
                    {ui(
                      "Requires Contents write permission in the GitHub App and installation. Saved edits are committed directly to the selected branch. Branch protection still applies. File paths are preserved; manifest formatting may be normalized.",
                    )}
                  </p>
                )}
              </div>
              {mode === "two_way" && github.app_settings_url && (
                <a
                  href={github.app_settings_url}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-sm underline"
                >
                  {ui("Open GitHub App settings")} <ExternalLink className="inline size-3" />
                </a>
              )}
              <div className="grid gap-4 sm:grid-cols-2">
                <div className="space-y-2">
                  <Label htmlFor="source-repository">{ui("Repository")}</Label>
                  <SharedSelect
                    value={String(location.repository)}
                    disabled={busy}
                    onValueChange={(selection) => {
                      const repo = selection;
                      setOptions(null);
                      perform(() => browse({ repository: repo, branch: "", path: "" }));
                    }}
                  >
                    <SharedSelectTrigger
                      id="source-repository"
                      className="h-9 w-full border bg-background px-3 text-sm"
                    >
                      <SharedSelectValue />
                    </SharedSelectTrigger>
                    <SharedSelectContent>
                      <SharedSelectItem value="">{ui("Select a repository")}</SharedSelectItem>
                      {repositories?.map((repo) => (
                        <SharedSelectItem key={repo.full_name} value={String(repo.full_name)}>
                          {repo.full_name}
                        </SharedSelectItem>
                      ))}
                    </SharedSelectContent>
                  </SharedSelect>
                  {repoError && (
                    <p role="alert" className="break-words text-sm text-destructive">
                      {ui("Could not list repositories. Check the GitHub connection.")}
                    </p>
                  )}
                </div>
                <div className="space-y-2">
                  <Label htmlFor="source-branch">{ui("Branch")}</Label>
                  <Input
                    id="source-branch"
                    list="source-branches"
                    value={location.branch}
                    disabled={busy || !location.repository}
                    onChange={(event) => {
                      change({ ...location, branch: event.target.value, path: "" });
                      setOptions(null);
                    }}
                    placeholder={ui("main")}
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
                  <Label htmlFor="source-path">{ui("Package folder or manifest")}</Label>
                  <div className="flex gap-2">
                    <Input
                      id="source-path"
                      value={location.path}
                      disabled={busy || !location.repository}
                      onChange={(event) => change({ ...location, path: event.target.value })}
                      placeholder={ui("agents/support or agents/support/agent.toml")}
                    />
                    <Button
                      variant="outline"
                      disabled={busy || !location.repository || !location.branch}
                      onClick={() => perform(() => browse(location))}
                    >
                      {ui("Browse folder")}
                    </Button>
                  </div>
                  <p className="text-xs text-muted-foreground">
                    {ui(
                      "Leave empty for the repository root. Folder packages include declared files and skills.",
                    )}
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
                          {ui("Parent folder")}
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
                {busy ? ui("Loading…") : ui("Preview changes")}
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
              {ui("Cancel")}
            </Button>
            <Button
              disabled={busy || !preview?.valid}
              onClick={() =>
                perform(async () => {
                  if (!preview?.valid) return;
                  await connectAgentSource(agentId, {
                    ...location,
                    mode,
                    commit: preview.commit,
                    agent_digest: preview.agent_digest,
                    ...(source ? { expected_generation: source.generation } : {}),
                  });
                  await refresh();
                  setOpen(false);
                })
              }
            >
              {busy ? ui("Applying…") : ui("Enable continuous sync")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <Dialog
        open={disconnectOpen && canManage}
        onOpenChange={(value) => {
          if (!busy) setDisconnectOpen(value);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{ui("Disconnect source sync?")}</DialogTitle>
            <DialogDescription>
              {ui(
                "Your current definition stays in Everruns and remains editable. New sync attempts stop. A GitHub write already in flight may still complete.",
              )}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" disabled={busy} onClick={() => setDisconnectOpen(false)}>
              {ui("Cancel")}
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
              {ui("Disconnect source sync")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
