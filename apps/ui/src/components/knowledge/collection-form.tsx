"use client";
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useModels } from "@/hooks/use-providers";
import { useOrg } from "@/providers/org-provider";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Label } from "@/components/ui/label";
import {
  listKnowledgeIndexers,
  type KnowledgeCollection,
  type CollectionInput,
  type IndexMode,
} from "@/lib/api/knowledge";

const selectClass = "h-9 w-full rounded-md border border-input bg-background px-3 text-sm";
export function CollectionForm({
  initial,
  onSave,
  onCancel,
}: {
  initial?: KnowledgeCollection;
  onSave: (input: CollectionInput) => Promise<unknown>;
  onCancel?: () => void;
}) {
  const [name, setName] = useState(initial?.name ?? "");
  const [description, setDescription] = useState(initial?.description ?? "");
  const [mode, setMode] = useState<IndexMode>(initial?.indexing ?? "off");
  const [model, setModel] = useState(
    initial?.embedding_model_id ? `model_${initial.embedding_model_id.replaceAll("-", "")}` : "",
  );
  const [indexer, setIndexer] = useState(initial?.indexer_id ?? "");
  const [rules, setRules] = useState(
    JSON.stringify(
      initial?.rules && Object.keys(initial.rules).length
        ? initial.rules
        : { ingest_exclude: [], index_exclude: [], read_exclude: [] },
      null,
      2,
    ),
  );
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const { data: models } = useModels();
  const { currentOrg } = useOrg();
  const accounts = useQuery({
    queryKey: ["knowledge-indexers", currentOrg?.public_id],
    queryFn: listKnowledgeIndexers,
    enabled: !!currentOrg,
  });
  return (
    <form
      className="space-y-4"
      onSubmit={async (e) => {
        e.preventDefault();
        setError("");
        setPending(true);
        try {
          await onSave({
            name,
            description,
            indexing: mode,
            embedding_model_id: mode === "hybrid" ? model : null,
            indexer_id: indexer || null,
            rules: JSON.parse(rules),
          });
        } catch (e) {
          setError(e instanceof Error ? e.message : "Unable to save collection");
        } finally {
          setPending(false);
        }
      }}
    >
      <div className="space-y-2">
        <Label htmlFor="knowledge-name">Name</Label>
        <Input
          id="knowledge-name"
          required
          maxLength={255}
          value={name}
          disabled={initial?.name === "everruns-docs"}
          onChange={(e) => setName(e.target.value)}
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor="knowledge-description">Description</Label>
        <Textarea
          id="knowledge-description"
          maxLength={4096}
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          placeholder="What should an agent use this collection for?"
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor="knowledge-indexing">Indexing</Label>
        <select
          id="knowledge-indexing"
          className={selectClass}
          value={mode}
          onChange={(e) => setMode(e.target.value as IndexMode)}
        >
          <option value="off">Off · browse, mount and grep</option>
          <option value="lexical">Keyword search</option>
          <option value="hybrid">Hybrid · keyword and semantic search</option>
        </select>
        <p className="text-xs text-muted-foreground">
          Original files stay available independently of indexing. Hybrid indexing uses your
          selected embedding provider.
        </p>
      </div>
      {mode !== "off" && (
        <div className="space-y-2">
          <Label htmlFor="knowledge-indexer">Search account</Label>
          <select
            id="knowledge-indexer"
            className={selectClass}
            value={indexer}
            onChange={(e) => setIndexer(e.target.value)}
          >
            <option value="">Deployment default</option>
            {accounts.data?.map((account) => (
              <option key={account.id} value={account.id}>
                {account.name}
              </option>
            ))}
          </select>
          <p className="text-xs text-muted-foreground">
            Keyword search works locally. Hybrid search requires a configured vector search account.
          </p>
        </div>
      )}
      {mode === "hybrid" && (
        <div className="space-y-2">
          <Label htmlFor="knowledge-model">Embedding model</Label>
          <select
            id="knowledge-model"
            className={selectClass}
            required
            value={model}
            onChange={(e) => setModel(e.target.value)}
          >
            <option value="">Select an embedding model</option>
            {models
              ?.filter((m) => m.enabled && m.service === "embeddings")
              .map((m) => (
                <option key={m.id} value={m.id}>
                  {m.provider_name} / {m.model_id}
                </option>
              ))}
          </select>
        </div>
      )}
      <details>
        <summary className="cursor-pointer text-sm">File exclusions</summary>
        <div className="mt-3 space-y-2">
          <Label htmlFor="knowledge-rules">Path rules</Label>
          <Textarea
            id="knowledge-rules"
            className="font-mono text-xs"
            rows={7}
            value={rules}
            onChange={(e) => setRules(e.target.value)}
          />
          <p className="text-xs text-muted-foreground">
            Ingestion omits files entirely; indexing controls search; read exclusions also hide
            files from agents.
          </p>
        </div>
      </details>
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      <div className="flex gap-2">
        <Button type="submit" disabled={pending} variant="accent">
          {pending ? "Saving…" : initial ? "Save settings" : "Create collection"}
        </Button>
        {onCancel && (
          <Button type="button" variant="outline" onClick={onCancel}>
            Cancel
          </Button>
        )}
      </div>
    </form>
  );
}
