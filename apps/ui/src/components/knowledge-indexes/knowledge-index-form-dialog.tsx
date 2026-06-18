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
import type {
  CreateKnowledgeIndexRequest,
  KnowledgeIndex,
  KnowledgeIndexSourceType,
  UpdateKnowledgeIndexRequest,
} from "@/lib/api/types";
import { EmbeddingModelPicker } from "./embedding-model-picker";

type FormMode = "create" | "edit";

interface KnowledgeIndexFormDialogProps {
  mode: FormMode;
  open: boolean;
  index?: KnowledgeIndex | null;
  isPending?: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (request: CreateKnowledgeIndexRequest | UpdateKnowledgeIndexRequest) => Promise<void>;
}

// source_config mirrors source-backed Memory's GitHub coordinates exactly so the
// same fields drive both. We render structured inputs (repository, branch, root
// folder) rather than raw JSON; the JSON object is assembled on submit.
function readStringField(config: Record<string, unknown> | undefined, key: string): string {
  const value = config?.[key];
  return typeof value === "string" ? value : "";
}

export function KnowledgeIndexFormDialog({
  mode,
  open,
  index,
  isPending = false,
  onOpenChange,
  onSubmit,
}: KnowledgeIndexFormDialogProps) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [sourceType, setSourceType] = useState<KnowledgeIndexSourceType>("github");
  const [repository, setRepository] = useState("");
  const [gitUrl, setGitUrl] = useState("");
  const [branch, setBranch] = useState("");
  const [rootFolder, setRootFolder] = useState("");
  const [embeddingModelId, setEmbeddingModelId] = useState("");
  const [fieldError, setFieldError] = useState<string | null>(null);
  const [formError, setFormError] = useState<string | null>(null);
  const { data: connections = [] } = useUserConnections();
  const githubConnection = connections.find((connection) => connection.provider === "github");

  useEffect(() => {
    if (!open) {
      return;
    }
    const config = (index?.source_config ?? {}) as Record<string, unknown>;
    setName(mode === "edit" ? (index?.name ?? "") : "");
    setDescription(mode === "edit" ? (index?.description ?? "") : "");
    setSourceType(index?.source_type === "git" ? "git" : "github");
    setRepository(readStringField(config, "repository"));
    setGitUrl(readStringField(config, "url"));
    setBranch(readStringField(config, "branch"));
    setRootFolder(readStringField(config, "root_folder"));
    setEmbeddingModelId(mode === "edit" ? (index?.embedding_model_id ?? "") : "");
    setFieldError(null);
    setFormError(null);
  }, [mode, open, index]);

  function buildSourceConfig(): Record<string, unknown> {
    const trimmedBranch = branch.trim();
    const trimmedRootFolder = rootFolder.trim();
    const base: Record<string, unknown> = {
      provider: sourceType,
      ...(trimmedBranch ? { branch: trimmedBranch } : {}),
      ...(trimmedRootFolder ? { root_folder: trimmedRootFolder } : {}),
    };
    if (sourceType === "github") {
      return { ...base, repository: repository.trim() };
    }
    return { ...base, url: gitUrl.trim() };
  }

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setFieldError(null);
    setFormError(null);

    const trimmedName = name.trim();
    if (!trimmedName) {
      setFieldError("Name is required");
      return;
    }
    if (!embeddingModelId) {
      setFieldError("An embedding model is required");
      return;
    }

    try {
      const trimmedDescription = description.trim();
      const sourceConfig = buildSourceConfig();
      if (mode === "create") {
        await onSubmit({
          name: trimmedName,
          ...(trimmedDescription ? { description: trimmedDescription } : {}),
          source_type: sourceType,
          source_config: sourceConfig,
          embedding_model_id: embeddingModelId,
        });
      } else {
        await onSubmit({
          name: trimmedName,
          description: trimmedDescription || null,
          source_config: sourceConfig,
          embedding_model_id: embeddingModelId,
        });
      }
      onOpenChange(false);
    } catch (error) {
      setFormError(error instanceof Error ? error.message : "Knowledge index could not be saved");
    }
  }

  const title = mode === "create" ? "New Knowledge Index" : "Edit Knowledge Index";
  const submitLabel = mode === "create" ? "Create Index" : "Save Changes";

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>
            {mode === "create" ? "Create an org-scoped knowledge index." : index?.id}
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="kidx-name">Name</Label>
            <Input
              id="kidx-name"
              value={name}
              onChange={(event) => {
                setName(event.target.value);
                setFieldError(null);
              }}
              aria-invalid={!!fieldError}
              required
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="kidx-description">Description</Label>
            <Textarea
              id="kidx-description"
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              rows={3}
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="kidx-embedding-model">Embedding model</Label>
            <EmbeddingModelPicker
              id="kidx-embedding-model"
              value={embeddingModelId}
              onChange={(modelId) => {
                setEmbeddingModelId(modelId);
                setFieldError(null);
              }}
            />
            <p className="text-xs text-muted-foreground">
              Required. Chunks are embedded with this model; it cannot be cleared.
            </p>
          </div>
          <div className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="kidx-source">Source</Label>
              <Select
                value={sourceType}
                onValueChange={(value) => setSourceType(value as KnowledgeIndexSourceType)}
              >
                <SelectTrigger id="kidx-source" className="w-full" aria-label="Source">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="github">GitHub</SelectItem>
                  <SelectItem value="git">Git URL</SelectItem>
                </SelectContent>
              </Select>
            </div>
            {sourceType === "github" && (
              <div className="grid gap-4 sm:grid-cols-2">
                <div className="space-y-2">
                  <Label htmlFor="kidx-repository">Repository</Label>
                  <Input
                    id="kidx-repository"
                    value={repository}
                    onChange={(event) => setRepository(event.target.value)}
                    placeholder="owner/repo"
                  />
                </div>
                <div className="space-y-2">
                  <Label htmlFor="kidx-github-connection">Connection</Label>
                  <Select value={githubConnection ? "github" : "none"} disabled>
                    <SelectTrigger
                      id="kidx-github-connection"
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
            {sourceType === "git" && (
              <div className="space-y-2">
                <Label htmlFor="kidx-git-url">Git URL</Label>
                <Input
                  id="kidx-git-url"
                  value={gitUrl}
                  onChange={(event) => setGitUrl(event.target.value)}
                  placeholder="https://example.com/org/repo.git"
                />
              </div>
            )}
            <div className="grid gap-4 sm:grid-cols-2">
              <div className="space-y-2">
                <Label htmlFor="kidx-branch">Branch</Label>
                <Input
                  id="kidx-branch"
                  value={branch}
                  onChange={(event) => setBranch(event.target.value)}
                  placeholder="main"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="kidx-root-folder">Root Folder</Label>
                <Input
                  id="kidx-root-folder"
                  value={rootFolder}
                  onChange={(event) => setRootFolder(event.target.value)}
                  placeholder="docs"
                />
              </div>
            </div>
          </div>
          {fieldError && <p className="text-xs text-destructive">{fieldError}</p>}
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
