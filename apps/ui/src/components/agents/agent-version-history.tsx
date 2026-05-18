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
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
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
  const [rollbackVersion, setRollbackVersion] = useState<AgentVersion | null>(null);
  const [forkVersion, setForkVersion] = useState<AgentVersion | null>(null);
  const [forkName, setForkName] = useState("");
  const [forkDisplayName, setForkDisplayName] = useState("");
  const [forkDescription, setForkDescription] = useState("");

  const createVersion = useCreateAgentVersion();
  const setDefault = useSetDefaultAgentVersion();
  const rollback = useRollbackAgentVersion();
  const fork = useForkAgentVersion();
  const diff = useAgentVersionDiff(agent.id, fromVersionId || undefined, toVersionId || undefined);
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
    () => versions.find((version) => version.id === fromVersionId),
    [fromVersionId, versions],
  );
  const selectedTo = useMemo(
    () => versions.find((version) => version.id === toVersionId),
    [toVersionId, versions],
  );

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
    <div className="grid gap-6 lg:grid-cols-[minmax(0,1fr)_360px]">
      <div className="space-y-6">
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <History className="h-4 w-4" />
              Version History
            </CardTitle>
          </CardHeader>
          <CardContent>
            {isLoading ? (
              <div className="space-y-2">
                <Skeleton className="h-14 w-full" />
                <Skeleton className="h-14 w-full" />
              </div>
            ) : publishedVersions.length === 0 ? (
              <p className="py-8 text-center text-sm text-muted-foreground">
                No saved versions yet.
              </p>
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
                        Default
                      </Button>
                      <Button
                        size="sm"
                        variant="outline"
                        onClick={() => setRollbackVersion(version)}
                      >
                        <RotateCcw className="mr-1 h-3 w-3" />
                        Rollback
                      </Button>
                      <Button size="sm" variant="outline" onClick={() => setForkVersion(version)}>
                        <GitBranch className="mr-1 h-3 w-3" />
                        Fork
                      </Button>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Automatic Snapshots</CardTitle>
          </CardHeader>
          <CardContent>
            {isLoading ? (
              <div className="space-y-2">
                <Skeleton className="h-14 w-full" />
                <Skeleton className="h-14 w-full" />
              </div>
            ) : draftSnapshots.length === 0 ? (
              <p className="py-8 text-center text-sm text-muted-foreground">
                No automatic snapshots yet.
              </p>
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
                        Rollback
                      </Button>
                      <Button size="sm" variant="outline" onClick={() => setForkVersion(version)}>
                        <GitBranch className="mr-1 h-3 w-3" />
                        Fork
                      </Button>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Compare Versions</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="grid gap-3 md:grid-cols-2">
              <div>
                <Label>From</Label>
                <VersionSelect
                  value={fromVersionId}
                  versions={versions}
                  onValueChange={setFromVersionId}
                />
              </div>
              <div>
                <Label>To</Label>
                <VersionSelect
                  value={toVersionId}
                  versions={versions}
                  onValueChange={setToVersionId}
                />
              </div>
            </div>
            {selectedFrom && selectedTo && diff.data && (
              <div className="space-y-5">
                <p className="text-sm text-muted-foreground">
                  Comparing {selectedFrom.version} to {selectedTo.version}
                </p>
                <DiffBlock title="Authored Configuration" diff={diff.data.authored_diff} />
                <DiffBlock title="Resolved Configuration" diff={diff.data.resolved_diff} />
              </div>
            )}
          </CardContent>
        </Card>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Save Version</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <div>
            <Label>Change type</Label>
            <Select
              value={changeKind}
              onValueChange={(value) => setChangeKind(value as AgentVersionChangeKind)}
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="manual">Manual</SelectItem>
                <SelectItem value="patch">Patch</SelectItem>
                <SelectItem value="minor">Minor</SelectItem>
                <SelectItem value="major">Major</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div>
            <Label htmlFor="version-summary">Summary</Label>
            <Textarea
              id="version-summary"
              value={summary}
              onChange={(event) => setSummary(event.target.value)}
              placeholder="What changed in this agent configuration?"
            />
          </div>
          <Button onClick={saveVersion} disabled={createVersion.isPending} className="w-full">
            <Save className="mr-2 h-4 w-4" />
            {createVersion.isPending ? "Saving..." : "Save Version"}
          </Button>
        </CardContent>
      </Card>

      <Dialog open={!!rollbackVersion} onOpenChange={(open) => !open && setRollbackVersion(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Rollback to {rollbackVersion?.version}</DialogTitle>
          </DialogHeader>
          <p className="text-sm text-muted-foreground">
            The editable agent draft will be replaced with this version. A rollback version will be
            saved so the history stays auditable.
          </p>
          <DialogFooter>
            <Button variant="outline" onClick={() => setRollbackVersion(null)}>
              Cancel
            </Button>
            <Button onClick={submitRollback} disabled={rollback.isPending}>
              {rollback.isPending ? "Rolling back..." : "Rollback"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={!!forkVersion} onOpenChange={(open) => !open && setForkVersion(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Fork {forkVersion?.version}</DialogTitle>
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
              {fork.isPending ? "Forking..." : "Fork Agent"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function VersionSelect({
  value,
  versions,
  onValueChange,
}: {
  value: string;
  versions: AgentVersion[];
  onValueChange: (value: string) => void;
}) {
  return (
    <Select value={value} onValueChange={onValueChange}>
      <SelectTrigger>
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
