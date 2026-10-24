"use client";
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
  return (
    <form
      className="space-y-4"
      onSubmit={async (e) => {
        e.preventDefault();
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
      <Button type="submit" variant="accent" disabled={pending}>
        {pending ? "Saving…" : "Add search account"}
      </Button>
    </form>
  );
}
