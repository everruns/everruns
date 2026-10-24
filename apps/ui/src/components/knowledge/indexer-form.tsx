"use client";

import { useTextPageFormDraft } from "@/components/chat-mode/use-text-page-form-draft";
import { isPublicDraftLocation } from "@/components/chat-mode/page-form-schemas";
import { useOrg } from "@/providers/org-provider";
import { useState } from "react";
import { createKnowledgeIndexer } from "@/lib/api/knowledge";
import { createScopedCredential, saveCredentialValue } from "@/lib/api/credential-bindings";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
export function IndexerForm({ onSaved }: { onSaved: () => Promise<unknown> }) {
  const [name, setName] = useState("");
  const [endpoint, setEndpoint] = useState("https://gcp-us-central1.turbopuffer.com");
  const [key, setKey] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const { currentOrg } = useOrg();
  const chatDraft = useTextPageFormDraft({
    fields: ["name", "endpoint"],
    values: { name, endpoint },
    entityType: "knowledge_indexer",
    formId: "search_account",
    label: "New search account",
    baseline: "new",
    dirty: !!name || endpoint !== "https://gcp-us-central1.turbopuffer.com",
    writable: !pending && ["owner", "admin"].includes(currentOrg?.role ?? ""),
    enabled: isPublicDraftLocation(endpoint),
    // The write-only API key stays exclusively in the manual credential input.
    apply: (next) => {
      if (!isPublicDraftLocation(next.endpoint))
        throw new Error("Use a public endpoint without credentials.");
      setName(next.name);
      setEndpoint(next.endpoint);
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
          const credential = await createScopedCredential(
            { scope: "organization", usage: "service" },
            {
              name: `KNOWLEDGE_${crypto.randomUUID().replaceAll("-", "").toUpperCase()}`,
              label: `Knowledge · ${name}`,
              binding: { url: endpoint, auth: { type: "bearer" } },
            },
          );
          await saveCredentialValue(credential.id, key, 1);
          setKey("");
          await createKnowledgeIndexer({ name, endpoint, credential_binding_id: credential.id });
          await onSaved();
          setName("");
        } catch (e) {
          setError(e instanceof Error ? e.message : "Unable to configure search account");
        } finally {
          setPending(false);
        }
      }}
    >
      {chatDraft.notice}
      <p className="text-sm text-muted-foreground">
        Use your organization&apos;s Turbopuffer account. The key is stored as a write-only
        organization credential. Embedding costs use the collection&apos;s separate model provider.
      </p>
      <div className="space-y-2">
        <Label htmlFor="indexer-name">Account name</Label>
        <Input
          id="indexer-name"
          value={name}
          onChange={(e) => setName(e.target.value)}
          required
          maxLength={255}
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor="indexer-region">Regional endpoint</Label>
        <Input
          id="indexer-region"
          type="url"
          value={endpoint}
          onChange={(e) => setEndpoint(e.target.value)}
          required
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor="indexer-key">API key</Label>
        <Input
          id="indexer-key"
          type="password"
          autoComplete="new-password"
          value={key}
          onChange={(e) => setKey(e.target.value)}
          required
        />
      </div>
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      <Button type="submit" variant="accent" disabled={pending || chatDraft.blocked}>
        {pending ? "Saving…" : "Add search account"}
      </Button>
    </form>
  );
}
