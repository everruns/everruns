"use client";

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
  const [fieldError, setFieldError] = useState<string | null>(null);
  const [formError, setFormError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) {
      return;
    }
    setName(mode === "edit" ? (volume?.name ?? "") : "");
    setDescription(mode === "edit" ? (volume?.description ?? "") : "");
    setSourceKind("manual");
    setRepository("");
    setGitUrl("");
    setBranch("");
    setRootFolder("");
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
        });
        await onSubmit({
          name: trimmedName,
          ...(trimmedDescription ? { description: trimmedDescription } : {}),
          ...(source ? { source } : {}),
        });
      } else {
        await onSubmit({
          name: trimmedName,
          description: trimmedDescription || null,
        });
      }
      onOpenChange(false);
    } catch (error) {
      setFormError(error instanceof Error ? error.message : "Volume could not be saved");
    }
  }

  const title = mode === "create" ? "New Volume" : "Edit Volume";
  const submitLabel = mode === "create" ? "Create Volume" : "Save Changes";

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
          {mode === "create" && (
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
                    <SelectItem value="manual">Manual</SelectItem>
                    <SelectItem value="github">GitHub</SelectItem>
                    <SelectItem value="git">Git URL</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              {sourceKind === "github" && (
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
                <div className="grid gap-4 sm:grid-cols-2">
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
}: {
  sourceKind: CreateSourceKind;
  repository: string;
  gitUrl: string;
  branch: string;
  rootFolder: string;
}): CreateVolumeRequest["source"] {
  const trimmedBranch = branch.trim();
  const trimmedRootFolder = rootFolder.trim();
  const optionalPathFields = {
    ...(trimmedBranch ? { branch: trimmedBranch } : {}),
    ...(trimmedRootFolder ? { root_folder: trimmedRootFolder } : {}),
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
