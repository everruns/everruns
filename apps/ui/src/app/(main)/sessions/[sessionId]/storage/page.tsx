"use client";

import { useEffect, useState } from "react";
import { api } from "@/lib/api/client";
import type { ListResponse, KeyValueInfo, SecretInfo } from "@/lib/api/types";
import { useSessionContext } from "../session-context";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { Key, Lock, Clock, AlertCircle } from "lucide-react";
import { useOrg } from "@/providers/org-provider";

export default function StoragePage() {
  const { sessionId } = useSessionContext();
  const { currentOrg } = useOrg();
  const [keyValues, setKeyValues] = useState<KeyValueInfo[]>([]);
  const [secrets, setSecrets] = useState<SecretInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    async function fetchStorage() {
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
        console.error("Failed to fetch storage:", err);
        setError(err instanceof Error ? err.message : "Failed to load storage data");
      } finally {
        setLoading(false);
      }
    }

    fetchStorage();
  }, [sessionId, currentOrg]);

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
                <div key={kv.key} className="border rounded-lg p-3 space-y-2">
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
            Encrypted secrets stored by the agent (values are hidden)
          </CardDescription>
        </CardHeader>
        <CardContent>
          {secrets.length === 0 ? (
            <p className="text-muted-foreground text-sm">No secrets stored yet.</p>
          ) : (
            <div className="space-y-3">
              {secrets.map((secret) => (
                <div
                  key={secret.name}
                  className="border rounded-lg p-3 flex items-center justify-between"
                >
                  <div className="flex items-center gap-2">
                    <Badge variant="secondary" className="font-mono">
                      {secret.name}
                    </Badge>
                    <span className="text-muted-foreground text-sm">(encrypted)</span>
                  </div>
                  <div className="flex items-center gap-1 text-xs text-muted-foreground">
                    <Clock className="h-3 w-3" />
                    {formatDate(secret.updated_at)}
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
