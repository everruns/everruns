"use client";

import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useOrgScopedQuery } from "@/hooks/create-crud-hooks";
import { api } from "@/lib/api/client";
import { useUiTranslations } from "@/providers/locale-provider";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";

type Worker = {
  endpoint_url: string;
  enabled: boolean;
  last_error: string | null;
  last_invoked_at: string | null;
};

export function ExternalWorker({ agentId, disabled }: { agentId: string; disabled: boolean }) {
  const ui = useUiTranslations();
  const cache = useQueryClient();
  const path = `/v1/agents/${encodeURIComponent(agentId)}/external-worker`;
  const key = ["external-worker", agentId];
  const worker = useOrgScopedQuery({
    queryKey: key,
    queryFn: async () => (await api.get<Worker | null>(path)).data,
    refetchInterval: 10000,
  });
  const [open, setOpen] = useState(false);
  const [url, setUrl] = useState("");
  const [secret, setSecret] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function save(revoke: boolean) {
    setPending(true);
    setError(null);
    try {
      if (revoke) await api.delete(path);
      else await api.post(path, { endpoint_url: url, endpoint_secret: secret });
      setSecret("");
      setOpen(false);
      await cache.invalidateQueries({ queryKey: key });
    } catch (e) {
      setError(e instanceof Error ? e.message : ui("External worker setup failed"));
    } finally {
      setPending(false);
    }
  }

  return (
    <section className="border border-border p-4 space-y-3" aria-label={ui("Execution")}>
      <div className="flex items-center gap-3">
        <h2 className="font-medium">{ui("Execution")}</h2>
        {!worker.isLoading && !worker.isError && (
          <Badge variant="secondary">
            {worker.data ? ui("External worker") : ui("Built-in runtime")}
          </Badge>
        )}
        {worker.data && !worker.data.enabled && <Badge variant="outline">{ui("Disabled")}</Badge>}
        <Button
          variant="outline"
          size="sm"
          disabled={disabled || pending || worker.isLoading || worker.isError}
          onClick={() => {
            setUrl(worker.data?.endpoint_url ?? "");
            setSecret("");
            setError(null);
            setOpen(!open);
          }}
        >
          {ui("Configure execution")}
        </Button>
      </div>
      {worker.isError && (
        <p role="alert" className="text-destructive">
          {ui("Unable to load execution settings")}
        </p>
      )}
      {worker.data && (
        <>
          <p className="text-sm text-muted-foreground break-all">{worker.data.endpoint_url}</p>
          <p className="text-sm text-muted-foreground">
            {ui(
              "This endpoint owns the model, prompt and tools. Agent editor settings are not applied to external execution.",
            )}
          </p>
          {worker.data.last_error && (
            <p role="alert" className="text-destructive">
              {worker.data.last_error}
            </p>
          )}
        </>
      )}
      {open && (
        <form
          className="space-y-3"
          onSubmit={(e) => {
            e.preventDefault();
            void save(false);
          }}
        >
          <p className="text-sm text-muted-foreground">
            {ui(
              "Deploy your worker first. Install a random endpoint secret in its environment, then enter it here to verify ownership.",
            )}
          </p>
          <div className="space-y-1">
            <Label htmlFor="external-endpoint">{ui("Worker HTTPS endpoint")}</Label>
            <Input
              id="external-endpoint"
              type="url"
              required
              value={url}
              onChange={(e) => setUrl(e.target.value)}
            />
          </div>
          <div className="space-y-1">
            <Label htmlFor="external-secret">{ui("Endpoint secret")}</Label>
            <Input
              id="external-secret"
              type="password"
              autoComplete="new-password"
              required
              minLength={32}
              maxLength={256}
              value={secret}
              onChange={(e) => setSecret(e.target.value)}
            />
          </div>
          <div className="flex gap-2">
            <Button type="submit" disabled={pending}>
              {ui("Verify and connect")}
            </Button>
            {worker.data?.enabled && (
              <Button
                type="button"
                variant="outline"
                disabled={pending}
                onClick={() => void save(true)}
              >
                {ui("Disable external worker")}
              </Button>
            )}
          </div>
        </form>
      )}
      {error && (
        <p role="alert" className="text-destructive">
          {error}
        </p>
      )}
    </section>
  );
}
