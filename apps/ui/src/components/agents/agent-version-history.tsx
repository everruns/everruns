"use client";

import { useMemo, useState } from "react";
import { useRouter } from "next/navigation";
import { GitBranch, History, RotateCcw, Save, Star } from "lucide-react";
import {
  useAgentVersionDiff,
  useAgentVersions,
  useCreateAgentVersion,
  useForkAgentVersion,
  useRollbackAgentVersion,
  useSetDefaultAgentVersion,
} from "@/hooks/use-agents";
import type { Agent, AgentVersion, AgentVersionChangeKind } from "@/lib/api/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";

function formatVersionDate(value: string) {
  return new Date(value).toLocaleString();
}

function diffRows(diff: Record<string, { from: unknown; to: unknown }>) {
  return Object.entries(diff);
}

function DiffBlock({
  title,
  diff,
}: {
  title: string;
  diff: Record<string, { from: unknown; to: unknown }>;
}) {
  const rows = diffRows(diff);
  return (
    <div className="space-y-2">
      <h3 className="text-sm font-medium">{title}</h3>
      {rows.length === 0 ? (
        <p className="text-sm text-muted-foreground">No changes.</p>
      ) : (
        <div className="overflow-x-auto border">
          <table className="w-full text-sm">
            <thead className="bg-muted/60">
              <tr>
                <th className="w-40 p-2 text-left font-medium">Field</th>
                <th className="p-2 text-left font-medium">Before</th>
                <th className="p-2 text-left font-medium">After</th>
              </tr>
            </thead>
            <tbody>
              {rows.map(([key, change]) => (
                <tr key={key} className="border-t align-top">
                  <td className="p-2 font-mono text-xs">{key}</td>
                  <td className="p-2">
                    <pre className="max-h-48 whitespace-pre-wrap break-words text-xs">
                      {JSON.stringify(change.from, null, 2)}
                    </pre>
                  </td>
                  <td className="p-2">
                    <pre className="max-h-48 whitespace-pre-wrap break-words text-xs">
                      {JSON.stringify(change.to, null, 2)}
                    </pre>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

export function AgentVersionHistory({ agent }: { agent: Agent }) {
  const router = useRouter();
  const { data: versions = [], isLoading } = useAgentVersions(agent.id);
  const [summary, setSummary] = useState("");
  const [changeKind, setChangeKind] = useState<AgentVersionChangeKind>("manual");
  const [fromVersionId, setFromVersionId] = useState<string>("");
  const [toVersionId, setToVersionId] = useState<string>("");
  const [comparedVersionIds, setComparedVersionIds] = useState<{
    from: string;
    to: string;
  } | null>(null);
  const [rollbackVersion, setRollbackVersion] = useState<AgentVersion | null>(null);
  const [forkVersion, setForkVersion] = useState<AgentVersion | null>(null);
  const [forkName, setForkName] = useState("");
  const [forkDisplayName, setForkDisplayName] = useState("");
  const [forkDescription, setForkDescription] = useState("");

  const createVersion = useCreateAgentVersion();
  const setDefault = useSetDefaultAgentVersion();
  const rollback = useRollbackAgentVersion();
  const fork = useForkAgentVersion();
  const diff = useAgentVersionDiff(agent.id, comparedVersionIds?.from, comparedVersionIds?.to);
  const publishedVersions = useMemo(
    () => versions.filter((version) => version.is_published),
    [versions],
  );
  const draftSnapshots = useMemo(
    () => versions.filter((version) => !version.is_published),
    [versions],
  );
  const latest = publishedVersions[0];

  const selectedFrom = useMemo(
    () => versions.find((version) => version.id === comparedVersionIds?.from),
    [comparedVersionIds?.from, versions],
  );
  const selectedTo = useMemo(
    () => versions.find((version) => version.id === comparedVersionIds?.to),
    [comparedVersionIds?.to, versions],
  );

  const selectFromVersion = (versionId: string) => {
    setFromVersionId(versionId);
    setComparedVersionIds(null);
  };

  const selectToVersion = (versionId: string) => {
    setToVersionId(versionId);
    setComparedVersionIds(null);
  };

  const saveVersion = async () => {
    await createVersion.mutateAsync({
      agentId: agent.id,
      request: {
        summary: summary.trim() || undefined,
        change_kind: changeKind,
      },
    });
    setSummary("");
    setChangeKind("manual");
  };

  const submitRollback = async () => {
    if (!rollbackVersion) return;
    await rollback.mutateAsync({
      agentId: agent.id,
      versionId: rollbackVersion.id,
      request: {
        save_version: true,
        summary: `Rollback to ${rollbackVersion.version}`,
      },
    });
    setRollbackVersion(null);
  };

  const submitFork = async () => {
    if (!forkVersion || !forkName.trim()) return;
    const forked = await fork.mutateAsync({
      agentId: agent.id,
      versionId: forkVersion.id,
      request: {
        name: forkName.trim(),
        display_name: forkDisplayName.trim() || undefined,
        description: forkDescription.trim() || undefined,
      },
    });
    setForkVersion(null);
    router.push(`/agents/${forked.id}`);
  };

  return (
    <div className="mx-auto w-full max-w-4xl space-y-6">
      <Card>
        <CardHeader>
          <CardTitle>
            {publishedVersions.length === 0 ? "Save your first version" : "Save a new version"}
          </CardTitle>
          <CardDescription>
            {publishedVersions.length === 0
              ? "Save the current configuration so you can return to it later."
              : "Create a restore point for the current configuration."}
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-end">
            <div>
              <Label htmlFor="version-summary">What changed? (optional)</Label>
              <Input
                id="version-summary"
                value={summary}
                onChange={(event) => setSummary(event.target.value)}
                placeholder="For example: Updated the instructions"
              />
            </div>
            <Button onClick={saveVersion} disabled={createVersion.isPending}>
              <Save className="mr-2 h-4 w-4" />
              {createVersion.isPending ? "Saving..." : "Save current version"}
            </Button>
          </div>
          <details className="text-sm">
            <summary className="w-fit cursor-pointer text-muted-foreground hover:text-foreground">
              Version options
            </summary>
            <div className="mt-3 max-w-xs space-y-1">
              <Label>Type of change</Label>
              <Select
                value={changeKind}
                onValueChange={(value) => setChangeKind(value as AgentVersionChangeKind)}
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="manual">Standard update</SelectItem>
                  <SelectItem value="patch">Bug fix</SelectItem>
                  <SelectItem value="minor">New feature</SelectItem>
                  <SelectItem value="major">Breaking change</SelectItem>
                </SelectContent>
              </Select>
              <p className="text-xs text-muted-foreground">
                This controls how the version number changes.
              </p>
            </div>
          </details>
        </CardContent>
      </Card>

      {publishedVersions.length > 0 && (
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <History className="h-4 w-4" />
              Saved versions
            </CardTitle>
            <CardDescription>Versions you chose to keep.</CardDescription>
          </CardHeader>
          <CardContent>
            {isLoading ? (
              <div className="space-y-2">
                <Skeleton className="h-14 w-full" />
                <Skeleton className="h-14 w-full" />
              </div>
            ) : (
              <div className="divide-y border">
                {publishedVersions.map((version) => (
                  <div key={version.id} className="grid gap-3 p-3 md:grid-cols-[1fr_auto]">
                    <div className="min-w-0 space-y-1">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="font-medium">{version.version}</span>
                        <Badge variant="outline">{version.change_kind}</Badge>
                        {version.id === latest?.id && <Badge>Latest</Badge>}
                        {version.id === agent.default_version_id && (
                          <Badge variant="secondary">Default</Badge>
                        )}
                      </div>
                      <p className="text-sm text-muted-foreground">
                        {version.summary || "No summary"}
                      </p>
                      <p className="font-mono text-xs text-muted-foreground">
                        {formatVersionDate(version.created_at)} · {version.config_hash.slice(0, 12)}
                      </p>
                    </div>
                    <div className="flex flex-wrap items-start gap-2">
                      <Button
                        size="sm"
                        variant="outline"
                        onClick={() =>
                          setDefault.mutate({
                            agentId: agent.id,
                            request: { version_id: version.id },
                          })
                        }
                        disabled={version.id === agent.default_version_id || setDefault.isPending}
                      >
                        <Star className="mr-1 h-3 w-3" />
                        Use by default
                      </Button>
                      <Button
                        size="sm"
                        variant="outline"
                        onClick={() => setRollbackVersion(version)}
                      >
                        <RotateCcw className="mr-1 h-3 w-3" />
                        Restore
                      </Button>
                      <Button size="sm" variant="outline" onClick={() => setForkVersion(version)}>
                        <GitBranch className="mr-1 h-3 w-3" />
                        Create copy
                      </Button>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </CardContent>
        </Card>
      )}

      {draftSnapshots.length > 0 && (
        <details className="border bg-card text-card-foreground">
          <summary className="cursor-pointer px-4 py-3 text-sm font-medium hover:bg-muted/40">
            Recovery snapshots ({draftSnapshots.length})
          </summary>
          <div className="space-y-3 border-t px-4 py-4">
            <p className="text-sm text-muted-foreground">
              Everruns saves these automatically in case you need to recover an earlier draft.
            </p>
            {isLoading ? (
              <div className="space-y-2">
                <Skeleton className="h-14 w-full" />
                <Skeleton className="h-14 w-full" />
              </div>
            ) : (
              <div className="divide-y border">
                {draftSnapshots.map((version) => (
                  <div key={version.id} className="grid gap-3 p-3 md:grid-cols-[1fr_auto]">
                    <div className="min-w-0 space-y-1">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="font-medium">{version.version}</span>
                        <Badge variant="outline">auto</Badge>
                      </div>
                      <p className="font-mono text-xs text-muted-foreground">
                        {formatVersionDate(version.created_at)} · {version.config_hash.slice(0, 12)}
                      </p>
                    </div>
                    <div className="flex flex-wrap items-start gap-2">
                      <Button
                        size="sm"
                        variant="outline"
                        onClick={() => setRollbackVersion(version)}
                      >
                        <RotateCcw className="mr-1 h-3 w-3" />
                        Restore
                      </Button>
                      <Button size="sm" variant="outline" onClick={() => setForkVersion(version)}>
                        <GitBranch className="mr-1 h-3 w-3" />
                        Create copy
                      </Button>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        </details>
      )}

      {versions.length >= 2 && (
        <details className="border bg-card text-card-foreground">
          <summary className="cursor-pointer px-4 py-3 text-sm font-medium hover:bg-muted/40">
            Compare versions
          </summary>
          <div className="space-y-4 border-t px-4 py-4">
            <div className="grid gap-3 md:grid-cols-2">
              <div>
                <Label>From</Label>
                <VersionSelect
                  label="From revision"
                  value={fromVersionId}
                  versions={versions}
                  onValueChange={selectFromVersion}
                />
              </div>
              <div>
                <Label>To</Label>
                <VersionSelect
                  label="To revision"
                  value={toVersionId}
                  versions={versions}
                  onValueChange={selectToVersion}
                />
              </div>
            </div>
            {fromVersionId && toVersionId && fromVersionId !== toVersionId && (
              <Button
                variant="outline"
                onClick={() => setComparedVersionIds({ from: fromVersionId, to: toVersionId })}
              >
                Compare revisions
              </Button>
            )}
            {selectedFrom && selectedTo && diff.data && (
              <div className="space-y-5">
                <p className="text-sm text-muted-foreground">
                  Comparing {selectedFrom.version} to {selectedTo.version}
                </p>
                <DiffBlock title="Authored Configuration" diff={diff.data.authored_diff} />
                <DiffBlock title="Resolved Configuration" diff={diff.data.resolved_diff} />
              </div>
            )}
          </div>
        </details>
      )}

      <Dialog open={!!rollbackVersion} onOpenChange={(open) => !open && setRollbackVersion(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Restore {rollbackVersion?.version}?</DialogTitle>
          </DialogHeader>
          <p className="text-sm text-muted-foreground">
            This replaces the current editable configuration. Everruns will save the current state
            first, so you can undo this restore later.
          </p>
          <DialogFooter>
            <Button variant="outline" onClick={() => setRollbackVersion(null)}>
              Cancel
            </Button>
            <Button onClick={submitRollback} disabled={rollback.isPending}>
              {rollback.isPending ? "Restoring..." : "Restore version"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={!!forkVersion} onOpenChange={(open) => !open && setForkVersion(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Create a copy of {forkVersion?.version}</DialogTitle>
          </DialogHeader>
          <div className="space-y-4">
            <div>
              <Label htmlFor="fork-name">Agent name</Label>
              <Input
                id="fork-name"
                value={forkName}
                onChange={(event) => setForkName(event.target.value)}
              />
            </div>
            <div>
              <Label htmlFor="fork-display-name">Display name</Label>
              <Input
                id="fork-display-name"
                value={forkDisplayName}
                onChange={(event) => setForkDisplayName(event.target.value)}
              />
            </div>
            <div>
              <Label htmlFor="fork-description">Description</Label>
              <Textarea
                id="fork-description"
                value={forkDescription}
                onChange={(event) => setForkDescription(event.target.value)}
              />
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setForkVersion(null)}>
              Cancel
            </Button>
            <Button onClick={submitFork} disabled={fork.isPending || !forkName.trim()}>
              {fork.isPending ? "Creating..." : "Create agent copy"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function VersionSelect({
  label,
  value,
  versions,
  onValueChange,
}: {
  label: string;
  value: string;
  versions: AgentVersion[];
  onValueChange: (value: string) => void;
}) {
  return (
    <Select value={value} onValueChange={onValueChange}>
      <SelectTrigger aria-label={label}>
        <SelectValue placeholder="Select version" />
      </SelectTrigger>
      <SelectContent>
        {versions.map((version) => (
          <SelectItem key={version.id} value={version.id}>
            {version.version}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}
