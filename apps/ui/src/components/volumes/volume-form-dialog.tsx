"use client";

import Link from "next/link";
import { useEffect, useState, type FormEvent } from "react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
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
import { Textarea } from "@/components/ui/textarea";
import { useUserConnections } from "@/hooks/use-user-connections";
import type { CreateVolumeRequest, UpdateVolumeRequest, Volume } from "@/lib/api/types";

type VolumeFormMode = "create" | "edit";
type CreateSourceKind = "manual" | "github" | "git";

interface VolumeFormDialogProps {
  mode: VolumeFormMode;
  open: boolean;
  volume?: Volume | null;
  isPending?: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (request: CreateVolumeRequest | UpdateVolumeRequest) => Promise<void>;
}

export function VolumeFormDialog({
  mode,
  open,
  volume,
  isPending = false,
  onOpenChange,
  onSubmit,
}: VolumeFormDialogProps) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [sourceKind, setSourceKind] = useState<CreateSourceKind>("manual");
  const [repository, setRepository] = useState("");
  const [gitUrl, setGitUrl] = useState("");
  const [branch, setBranch] = useState("");
  const [rootFolder, setRootFolder] = useState("");
  const [syncIntervalSecs, setSyncIntervalSecs] = useState("0");
  const [fieldError, setFieldError] = useState<string | null>(null);
  const [formError, setFormError] = useState<string | null>(null);
  const { data: connections = [] } = useUserConnections();
  const githubConnection = connections.find((connection) => connection.provider === "github");

  useEffect(() => {
    if (!open) {
      return;
    }
    setName(mode === "edit" ? (volume?.name ?? "") : "");
    setDescription(mode === "edit" ? (volume?.description ?? "") : "");
    setSourceKind(
      volume?.source.provider === "git"
        ? "git"
        : volume?.source.provider === "github"
          ? "github"
          : "manual",
    );
    setRepository("");
    setGitUrl("");
    setBranch("");
    setRootFolder("");
    setSyncIntervalSecs("0");
    if (mode === "edit" && volume?.source.provider === "github") {
      setRepository(volume.source.repository);
      setBranch(volume.source.branch);
      setRootFolder(volume.source.root_folder ?? "");
      setSyncIntervalSecs(String(volume.source.sync_interval_secs ?? 0));
    }
    if (mode === "edit" && volume?.source.provider === "git") {
      setGitUrl(volume.source.url);
      setBranch(volume.source.branch);
      setRootFolder(volume.source.root_folder ?? "");
      setSyncIntervalSecs(String(volume.source.sync_interval_secs ?? 0));
    }
    setFieldError(null);
    setFormError(null);
  }, [mode, open, volume]);

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setFieldError(null);
    setFormError(null);

    const trimmedName = name.trim();
    if (!trimmedName) {
      setFieldError("Name is required");
      return;
    }

    try {
      const trimmedDescription = description.trim();
      if (mode === "create") {
        const source = buildCreateSource({
          sourceKind,
          repository,
          gitUrl,
          branch,
          rootFolder,
          syncIntervalSecs,
        });
        await onSubmit({
          name: trimmedName,
          ...(trimmedDescription ? { description: trimmedDescription } : {}),
          ...(source ? { source } : {}),
        });
      } else {
        const source =
          volume?.source.provider === "manual"
            ? undefined
            : buildCreateSource({
                sourceKind,
                repository,
                gitUrl,
                branch,
                rootFolder,
                syncIntervalSecs,
              });
        await onSubmit({
          name: trimmedName,
          description: trimmedDescription || null,
          ...(source && volume && !sourceMatchesVolume(volume, source) ? { source } : {}),
        });
      }
      onOpenChange(false);
    } catch (error) {
      setFormError(error instanceof Error ? error.message : "Volume could not be saved");
    }
  }

  const title = mode === "create" ? "New Volume" : "Edit Volume";
  const submitLabel = mode === "create" ? "Create Volume" : "Save Changes";
  const showSourceFields = mode === "create" || volume?.source.provider !== "manual";

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>
            {mode === "create" ? "Create an org-scoped workspace volume." : volume?.id}
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="volume-name">Name</Label>
            <Input
              id="volume-name"
              value={name}
              onChange={(event) => {
                setName(event.target.value);
                setFieldError(null);
              }}
              aria-invalid={!!fieldError}
              required
            />
            {fieldError && <p className="text-xs text-destructive">{fieldError}</p>}
          </div>
          <div className="space-y-2">
            <Label htmlFor="volume-description">Description</Label>
            <Textarea
              id="volume-description"
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              rows={3}
            />
          </div>
          {showSourceFields && (
            <div className="space-y-4">
              <div className="space-y-2">
                <Label htmlFor="volume-source">Source</Label>
                <Select
                  value={sourceKind}
                  onValueChange={(value) => setSourceKind(value as CreateSourceKind)}
                >
                  <SelectTrigger id="volume-source" className="w-full" aria-label="Source">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {mode === "create" && <SelectItem value="manual">Manual</SelectItem>}
                    <SelectItem value="github">GitHub</SelectItem>
                    <SelectItem value="git">Git URL</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              {sourceKind === "github" && (
                <div className="grid gap-4 sm:grid-cols-2">
                  <div className="space-y-2">
                    <Label htmlFor="volume-repository">Repository</Label>
                    <Input
                      id="volume-repository"
                      value={repository}
                      onChange={(event) => setRepository(event.target.value)}
                      placeholder="owner/repo"
                      required
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="volume-github-connection">Connection</Label>
                    <Select value={githubConnection ? "github" : "none"} disabled>
                      <SelectTrigger
                        id="volume-github-connection"
                        className="w-full"
                        aria-label="GitHub connection"
                      >
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="github">
                          {githubConnection?.provider_username ?? "GitHub connected"}
                        </SelectItem>
                        <SelectItem value="none">No GitHub connection</SelectItem>
                      </SelectContent>
                    </Select>
                    {!githubConnection && (
                      <p className="text-xs text-muted-foreground">
                        Private repositories need a{" "}
                        <Link href="/settings/connections" className="underline">
                          GitHub connection
                        </Link>
                        .
                      </p>
                    )}
                  </div>
                </div>
              )}
              {sourceKind === "git" && (
                <div className="space-y-2">
                  <Label htmlFor="volume-git-url">Git URL</Label>
                  <Input
                    id="volume-git-url"
                    value={gitUrl}
                    onChange={(event) => setGitUrl(event.target.value)}
                    placeholder="https://example.com/org/repo.git"
                    required
                  />
                </div>
              )}
              {sourceKind !== "manual" && (
                <div className="grid gap-4 sm:grid-cols-3">
                  <div className="space-y-2">
                    <Label htmlFor="volume-branch">Branch</Label>
                    <Input
                      id="volume-branch"
                      value={branch}
                      onChange={(event) => setBranch(event.target.value)}
                      placeholder="main"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="volume-root-folder">Root Folder</Label>
                    <Input
                      id="volume-root-folder"
                      value={rootFolder}
                      onChange={(event) => setRootFolder(event.target.value)}
                      placeholder="docs"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="volume-sync-interval">Resync</Label>
                    <Select value={syncIntervalSecs} onValueChange={setSyncIntervalSecs}>
                      <SelectTrigger
                        id="volume-sync-interval"
                        className="w-full"
                        aria-label="Resync interval"
                      >
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="0">Manual only</SelectItem>
                        <SelectItem value="300">Every 5 minutes</SelectItem>
                        <SelectItem value="900">Every 15 minutes</SelectItem>
                        <SelectItem value="3600">Hourly</SelectItem>
                        <SelectItem value="86400">Daily</SelectItem>
                        <SelectItem value="604800">Weekly</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                </div>
              )}
            </div>
          )}
          {formError && <p className="text-sm text-destructive">{formError}</p>}
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
              disabled={isPending}
            >
              Cancel
            </Button>
            <Button type="submit" variant="accent" disabled={isPending}>
              {submitLabel}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function buildCreateSource({
  sourceKind,
  repository,
  gitUrl,
  branch,
  rootFolder,
  syncIntervalSecs,
}: {
  sourceKind: CreateSourceKind;
  repository: string;
  gitUrl: string;
  branch: string;
  rootFolder: string;
  syncIntervalSecs: string;
}): CreateVolumeRequest["source"] {
  const trimmedBranch = branch.trim();
  const trimmedRootFolder = rootFolder.trim();
  const parsedSyncInterval = Number.parseInt(syncIntervalSecs, 10);
  const optionalPathFields = {
    ...(trimmedBranch ? { branch: trimmedBranch } : {}),
    ...(trimmedRootFolder ? { root_folder: trimmedRootFolder } : {}),
    ...(parsedSyncInterval > 0 ? { sync_interval_secs: parsedSyncInterval } : {}),
  };
  if (sourceKind === "github") {
    return {
      type: "github",
      repository: repository.trim(),
      ...optionalPathFields,
    };
  }
  if (sourceKind === "git") {
    return {
      type: "git",
      url: gitUrl.trim(),
      ...optionalPathFields,
    };
  }
  return undefined;
}

function sourceMatchesVolume(volume: Volume, source: NonNullable<CreateVolumeRequest["source"]>) {
  if (source.type !== volume.source.provider) {
    return false;
  }
  const expectedBranch = source.branch ?? "main";
  const expectedRootFolder = source.root_folder ?? null;
  const expectedSyncInterval = source.sync_interval_secs ?? null;

  if (source.type === "github" && volume.source.provider === "github") {
    return (
      source.repository === volume.source.repository &&
      expectedBranch === volume.source.branch &&
      expectedRootFolder === (volume.source.root_folder ?? null) &&
      expectedSyncInterval === (volume.source.sync_interval_secs ?? null)
    );
  }

  if (source.type === "git" && volume.source.provider === "git") {
    return (
      source.url === volume.source.url &&
      expectedBranch === volume.source.branch &&
      expectedRootFolder === (volume.source.root_folder ?? null) &&
      expectedSyncInterval === (volume.source.sync_interval_secs ?? null)
    );
  }

  return false;
}
