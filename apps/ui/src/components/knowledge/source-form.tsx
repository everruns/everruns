"use client";

import { usePageFormDraft } from "@/components/chat-mode/use-page-form-draft";
import { sourceDraftSchema, isPublicDraftLocation } from "@/components/chat-mode/page-form-schemas";
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useOrg } from "@/providers/org-provider";
import { listVirtualUsers } from "@/lib/api/virtual-users";
import { addKnowledgeSource } from "@/lib/api/knowledge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
export function SourceForm({
  collectionId,
  onSaved,
}: {
  collectionId: string;
  onSaved: () => Promise<unknown>;
}) {
  const [kind, setKind] = useState("github");
  const [coordinates, setCoordinates] = useState("");
  const [branch, setBranch] = useState("main");
  const [folder, setFolder] = useState("");
  const [root, setRoot] = useState("sources");
  const [identity, setIdentity] = useState("");
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);
  const { currentOrg } = useOrg();
  const identities = useQuery({
    queryKey: ["knowledge-source-identities", currentOrg?.public_id],
    queryFn: () => listVirtualUsers(false, { usage: "service" }),
    enabled: !!currentOrg,
  });
  const values = { kind: kind as "github" | "git", coordinates, branch, folder, root, identity };
  const chatDraft = usePageFormDraft({
    schema: sourceDraftSchema,
    values,
    entityId: collectionId,
    entityType: "knowledge_collection",
    formId: "source",
    label: "Connect source",
    baseline: "new",
    dirty: !!(
      coordinates ||
      folder ||
      identity ||
      branch !== "main" ||
      root !== "sources" ||
      kind !== "github"
    ),
    writable: !pending && ["owner", "admin"].includes(currentOrg?.role ?? ""),
    enabled: !coordinates.includes("://") || isPublicDraftLocation(coordinates),
    apply: (next) => {
      setKind(next.kind);
      setCoordinates(next.coordinates);
      setBranch(next.branch);
      setFolder(next.folder);
      setRoot(next.root);
      setIdentity(next.identity);
    },
  });
  return (
    <form
      className="space-y-4"
      onSubmit={async (e) => {
        e.preventDefault();
        if (chatDraft.blocked) return;
        setPending(true);
        setError("");
        try {
          await addKnowledgeSource(collectionId, {
            kind,
            root_path: root,
            config: {
              [kind === "github" ? "repository" : "url"]: coordinates,
              branch,
              ...(folder ? { root_folder: folder } : {}),
            },
            ...(identity ? { connection_owner: identity } : {}),
          });
          await onSaved();
          setCoordinates("");
        } catch (e) {
          setError(e instanceof Error ? e.message : "Unable to connect source");
        } finally {
          setPending(false);
        }
      }}
    >
      {chatDraft.notice}
      <p className="text-sm text-muted-foreground">
        Connected files are read-only. Refreshing keeps their file structure and replaces only
        changed content.
      </p>
      <div className="space-y-2">
        <Label htmlFor="source-kind">Source</Label>
        <select
          id="source-kind"
          className="h-9 w-full rounded-md border bg-background px-3 text-sm"
          value={kind}
          onChange={(e) => setKind(e.target.value)}
        >
          <option value="github">GitHub</option>
          <option value="git">Public Git repository</option>
        </select>
      </div>
      <div className="space-y-2">
        <Label htmlFor="source-repository">
          {kind === "github" ? "Repository" : "Repository URL"}
        </Label>
        <Input
          id="source-repository"
          required
          value={coordinates}
          onChange={(e) => setCoordinates(e.target.value)}
          placeholder={kind === "github" ? "owner/repository" : "https://example.com/team/docs.git"}
        />
      </div>
      {kind === "github" && (
        <div className="space-y-2">
          <Label htmlFor="source-identity">Read as</Label>
          <select
            id="source-identity"
            className="h-9 w-full rounded-md border bg-background px-3 text-sm"
            value={identity}
            onChange={(e) => setIdentity(e.target.value)}
          >
            <option value="">Organization GitHub connection</option>
            {identities.data?.data.map((v) => (
              <option key={v.id} value={toUuid(v.id)}>
                {v.name}
              </option>
            ))}
          </select>
          <p className="text-xs text-muted-foreground">
            Uses the existing GitHub connection and its selected repositories. This collection is
            available to agents you attach it to in this organization.
          </p>
        </div>
      )}
      <div className="grid gap-3 sm:grid-cols-3">
        <div className="space-y-2">
          <Label htmlFor="source-branch">Branch</Label>
          <Input
            id="source-branch"
            value={branch}
            onChange={(e) => setBranch(e.target.value)}
            required
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor="source-folder">Source folder</Label>
          <Input
            id="source-folder"
            value={folder}
            onChange={(e) => setFolder(e.target.value)}
            placeholder="Whole repository"
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor="source-root">Collection folder</Label>
          <Input id="source-root" value={root} onChange={(e) => setRoot(e.target.value)} />
        </div>
      </div>
      {kind === "github" && (
        <p className="text-xs text-muted-foreground">
          Attached agents can read retained files. Disconnecting GitHub stops refreshes and keeps
          existing copies; deny file access or archive the collection to revoke them.
        </p>
      )}
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      <Button type="submit" variant="accent" disabled={pending || chatDraft.blocked}>
        {pending ? "Connecting…" : "Connect source"}
      </Button>
    </form>
  );
}
function toUuid(value: string) {
  const hex = value.includes("_") ? value.slice(value.indexOf("_") + 1) : value;
  return hex.length === 32
    ? `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`
    : hex;
}
