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
import type { CreateMemoryRequest, UpdateMemoryRequest, Memory } from "@/lib/api/types";

type MemoryFormMode = "create" | "edit";
type CreateSourceKind = "manual" | "github" | "git";

interface MemoryFormDialogProps {
  mode: MemoryFormMode;
  open: boolean;
  memory?: Memory | null;
  isPending?: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (request: CreateMemoryRequest | UpdateMemoryRequest) => Promise<void>;
}

export function MemoryFormDialog({
  mode,
  open,
  memory,
  isPending = false,
  onOpenChange,
  onSubmit,
}: MemoryFormDialogProps) {
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
    setName(mode === "edit" ? (memory?.name ?? "") : "");
    setDescription(mode === "edit" ? (memory?.description ?? "") : "");
    setSourceKind(
      memory?.source.provider === "git"
        ? "git"
        : memory?.source.provider === "github"
          ? "github"
          : "manual",
    );
    setRepository("");
    setGitUrl("");
    setBranch("");
    setRootFolder("");
    setSyncIntervalSecs("0");
    if (mode === "edit" && memory?.source.provider === "github") {
      setRepository(memory.source.repository);
      setBranch(memory.source.branch);
      setRootFolder(memory.source.root_folder ?? "");
      setSyncIntervalSecs(String(memory.source.sync_interval_secs ?? 0));
    }
    if (mode === "edit" && memory?.source.provider === "git") {
      setGitUrl(memory.source.url);
      setBranch(memory.source.branch);
      setRootFolder(memory.source.root_folder ?? "");
      setSyncIntervalSecs(String(memory.source.sync_interval_secs ?? 0));
    }
    setFieldError(null);
    setFormError(null);
  }, [mode, open, memory]);

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
          memory?.source.provider === "manual"
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
          ...(source && memory && !sourceMatchesMemory(memory, source) ? { source } : {}),
        });
      }
      onOpenChange(false);
    } catch (error) {
      setFormError(error instanceof Error ? error.message : "Memory could not be saved");
    }
  }

  const title = mode === "create" ? "New Memory" : "Edit Memory";
  const submitLabel = mode === "create" ? "Create Memory" : "Save Changes";
  const showSourceFields = mode === "create" || memory?.source.provider !== "manual";

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>
            {mode === "create" ? "Create an org-scoped memory." : memory?.id}
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="memory-name">Name</Label>
            <Input
              id="memory-name"
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
            <Label htmlFor="memory-description">Description</Label>
            <Textarea
              id="memory-description"
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              rows={3}
            />
          </div>
          {showSourceFields && (
            <div className="space-y-4">
              <div className="space-y-2">
                <Label htmlFor="memory-source">Source</Label>
                <Select
                  value={sourceKind}
                  onValueChange={(value) => setSourceKind(value as CreateSourceKind)}
                >
                  <SelectTrigger id="memory-source" className="w-full" aria-label="Source">
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
                    <Label htmlFor="memory-repository">Repository</Label>
                    <Input
                      id="memory-repository"
                      value={repository}
                      onChange={(event) => setRepository(event.target.value)}
                      placeholder="owner/repo"
                      required
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="memory-github-connection">Connection</Label>
                    <Select value={githubConnection ? "github" : "none"} disabled>
                      <SelectTrigger
                        id="memory-github-connection"
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
                  <Label htmlFor="memory-git-url">Git URL</Label>
                  <Input
                    id="memory-git-url"
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
                    <Label htmlFor="memory-branch">Branch</Label>
                    <Input
                      id="memory-branch"
                      value={branch}
                      onChange={(event) => setBranch(event.target.value)}
                      placeholder="main"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="memory-root-folder">Root Folder</Label>
                    <Input
                      id="memory-root-folder"
                      value={rootFolder}
                      onChange={(event) => setRootFolder(event.target.value)}
                      placeholder="docs"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="memory-sync-interval">Resync</Label>
                    <Select value={syncIntervalSecs} onValueChange={setSyncIntervalSecs}>
                      <SelectTrigger
                        id="memory-sync-interval"
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
}): CreateMemoryRequest["source"] {
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

function sourceMatchesMemory(memory: Memory, source: NonNullable<CreateMemoryRequest["source"]>) {
  if (source.type !== memory.source.provider) {
    return false;
  }
  const expectedBranch = source.branch ?? "main";
  const expectedRootFolder = source.root_folder ?? null;
  const expectedSyncInterval = source.sync_interval_secs ?? null;

  if (source.type === "github" && memory.source.provider === "github") {
    return (
      source.repository === memory.source.repository &&
      expectedBranch === memory.source.branch &&
      expectedRootFolder === (memory.source.root_folder ?? null) &&
      expectedSyncInterval === (memory.source.sync_interval_secs ?? null)
    );
  }

  if (source.type === "git" && memory.source.provider === "git") {
    return (
      source.url === memory.source.url &&
      expectedBranch === memory.source.branch &&
      expectedRootFolder === (memory.source.root_folder ?? null) &&
      expectedSyncInterval === (memory.source.sync_interval_secs ?? null)
    );
  }

  return false;
}
