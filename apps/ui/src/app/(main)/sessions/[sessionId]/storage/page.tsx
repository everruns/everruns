"use client";

import { FormEvent, useCallback, useEffect, useState } from "react";
import { api } from "@/lib/api/client";
import type { ListResponse, KeyValueInfo, SecretInfo } from "@/lib/api/types";
import { useSessionContext } from "../session-context";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { Key, Lock, Clock, AlertCircle, Plus, RefreshCw, Trash2 } from "lucide-react";
import { useOrg } from "@/providers/org-provider";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

export default function StoragePage() {
  const { sessionId } = useSessionContext();
  const { currentOrg } = useOrg();
  const [keyValues, setKeyValues] = useState<KeyValueInfo[]>([]);
  const [secrets, setSecrets] = useState<SecretInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [secretName, setSecretName] = useState("");
  const [secretValue, setSecretValue] = useState("");
  const [editingName, setEditingName] = useState<string | null>(null);
  const [savingSecret, setSavingSecret] = useState(false);

  const fetchStorage = useCallback(async () => {
    if (!currentOrg) return;

    setLoading(true);
    setError(null);

    try {
      const [keysRes, secretsRes] = await Promise.all([
        api.get<ListResponse<KeyValueInfo>>(`/v1/sessions/${sessionId}/storage/keys`),
        api.get<ListResponse<SecretInfo>>(`/v1/sessions/${sessionId}/storage/secrets`),
      ]);

      setKeyValues(keysRes.data.data);
      setSecrets(secretsRes.data.data);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load storage data");
    } finally {
      setLoading(false);
    }
  }, [sessionId, currentOrg]);

  useEffect(() => {
    void fetchStorage();
  }, [fetchStorage]);

  const saveSecret = async (event: FormEvent) => {
    event.preventDefault();
    const name = (editingName ?? secretName).trim();
    if (!name || !secretValue) return;
    setSavingSecret(true);
    setError(null);
    try {
      await api.put(`/v1/sessions/${sessionId}/storage/secrets`, {
        secrets: { [name]: secretValue },
      });
      setSecretName("");
      setEditingName(null);
      await fetchStorage();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to save secret");
    } finally {
      setSecretValue("");
      setSavingSecret(false);
    }
  };

  const deleteSecret = async (name: string) => {
    if (!confirm(`Delete ${name}? The value cannot be recovered.`)) return;
    try {
      await api.delete(`/v1/sessions/${sessionId}/storage/secrets/${encodeURIComponent(name)}`);
      await fetchStorage();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to delete secret");
    }
  };

  if (loading) {
    return (
      <div className="flex-1 p-6 space-y-6">
        <Skeleton className="h-48 w-full" />
        <Skeleton className="h-48 w-full" />
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex-1 p-6">
        <div className="flex items-center gap-2 text-destructive">
          <AlertCircle className="h-5 w-5" />
          <span>{error}</span>
        </div>
      </div>
    );
  }

  const formatDate = (dateString: string) => {
    return new Date(dateString).toLocaleString();
  };

  return (
    <div className="flex-1 p-6 space-y-6 overflow-y-auto">
      {/* Key-Value Storage */}
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Key className="h-5 w-5" />
            Key-Value Storage
          </CardTitle>
          <CardDescription>Session-scoped key-value pairs stored by the agent</CardDescription>
        </CardHeader>
        <CardContent>
          {keyValues.length === 0 ? (
            <p className="text-muted-foreground text-sm">No key-value pairs stored yet.</p>
          ) : (
            <div className="space-y-3">
              {keyValues.map((kv) => (
                <div key={kv.key} className="border p-3 space-y-2">
                  <div className="flex items-center justify-between">
                    <Badge variant="outline" className="font-mono">
                      {kv.key}
                    </Badge>
                    <div className="flex items-center gap-1 text-xs text-muted-foreground">
                      <Clock className="h-3 w-3" />
                      {formatDate(kv.updated_at)}
                    </div>
                  </div>
                  <pre className="text-sm bg-muted p-2 rounded overflow-x-auto whitespace-pre-wrap break-all">
                    {kv.value}
                  </pre>
                </div>
              ))}
            </div>
          )}
        </CardContent>
      </Card>

      {/* Secrets Storage */}
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Lock className="h-5 w-5" />
            Secrets Storage
          </CardTitle>
          <CardDescription>
            Session-scoped encrypted values. They last only for this session and are readable by the
            session&apos;s <span className="font-mono">secret_store</span> tool. For credentials
            used by scheduled or per-invocation Agent runs, use the Agent&apos;s Credentials tab.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <form
            onSubmit={saveSecret}
            className="grid gap-3 border bg-muted/30 p-4 sm:grid-cols-[1fr_1fr_auto] sm:items-end"
            autoComplete="off"
          >
            <div className="space-y-1.5">
              <Label htmlFor="session-secret-name">Name</Label>
              <Input
                id="session-secret-name"
                value={editingName ?? secretName}
                onChange={(event) => (editingName ? undefined : setSecretName(event.target.value))}
                disabled={editingName !== null}
                placeholder="API_KEY"
                required
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="session-secret-value">
                {editingName ? "Replacement value" : "Value"}
              </Label>
              <Input
                id="session-secret-value"
                name="session-secret-value"
                type="password"
                value={secretValue}
                onChange={(event) => setSecretValue(event.target.value)}
                autoComplete="new-password"
                autoCapitalize="none"
                autoCorrect="off"
                spellCheck={false}
                maxLength={64 * 1024}
                placeholder="Never shown again"
                required
              />
            </div>
            <div className="flex gap-2">
              <Button
                type="submit"
                disabled={savingSecret || !secretValue || !(editingName ?? secretName).trim()}
              >
                {editingName ? <RefreshCw className="size-4" /> : <Plus className="size-4" />}
                {savingSecret ? "Saving…" : editingName ? "Replace" : "Add"}
              </Button>
              {editingName && (
                <Button
                  type="button"
                  variant="ghost"
                  onClick={() => {
                    setEditingName(null);
                    setSecretValue("");
                  }}
                >
                  Cancel
                </Button>
              )}
            </div>
          </form>
          {secrets.length === 0 ? (
            <p className="text-muted-foreground text-sm">No secrets stored yet.</p>
          ) : (
            <div className="space-y-3">
              {secrets.map((secret) => (
                <div
                  key={secret.name}
                  className="border p-3 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between"
                >
                  <div className="flex items-center gap-2">
                    <Badge variant="secondary" className="font-mono">
                      {secret.name}
                    </Badge>
                    <span className="text-muted-foreground text-sm">(encrypted)</span>
                  </div>
                  <div className="flex items-center gap-2">
                    <div className="flex items-center gap-1 text-xs text-muted-foreground">
                      <Clock className="h-3 w-3" />
                      {formatDate(secret.updated_at)}
                    </div>
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => {
                        setEditingName(secret.name);
                        setSecretValue("");
                      }}
                    >
                      <RefreshCw className="size-4" /> Replace
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="text-destructive"
                      aria-label={`Delete ${secret.name}`}
                      onClick={() => deleteSecret(secret.name)}
                    >
                      <Trash2 className="size-4" />
                    </Button>
                  </div>
                </div>
              ))}
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
