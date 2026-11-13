"use client";
import { useState } from "react";
import { useUiTranslations } from "@/providers/locale-provider";
import {
  saveTracingDestination,
  type TracingConfig,
  type TracingDestination,
} from "@/lib/api/tracing";
import { tracingAuthHeaders, tracingProviders } from "./provider-config";
import { useRefreshTracing } from "@/hooks/use-tracing";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from "@/components/ui/select";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";

const empty: TracingConfig = {
  name: "",
  provider: "braintrust",
  endpoint: "https://api.braintrust.dev",
  project_id: null,
  project_name: null,
  langsmith_agent_id: null,
  langsmith_agent_environment: null,
  capture_content: false,
  capture_reasoning: false,
  enabled: true,
};
export function TracingDestinationDialog({
  destination,
  onClose,
  onCreated,
}: {
  destination?: TracingDestination;
  onClose: () => void;
  onCreated: () => void;
}) {
  const ui = useUiTranslations();
  const refresh = useRefreshTracing();
  const [config, setConfig] = useState<TracingConfig>(destination?.config ?? empty);
  const [secret, setSecret] = useState("");
  const [publicKey, setPublicKey] = useState("");
  const [header, setHeader] = useState("Authorization");
  const [replaceAuth, setReplaceAuth] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const receiverChanged =
    !!destination &&
    (config.provider !== destination.config.provider ||
      config.endpoint !== destination.config.endpoint);
  const needsSecret = !destination || replaceAuth || receiverChanged;
  const set = (value: Partial<TracingConfig>) => setConfig((c) => ({ ...c, ...value }));
  async function save(event: React.FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await saveTracingDestination(
        {
          config: {
            ...config,
            name: config.name.trim(),
            project_name: config.project_name?.trim() || null,
          },
          revision: destination?.revision,
          ...(needsSecret
            ? {
                secrets: {
                  headers: tracingAuthHeaders(config.provider, secret, publicKey, header),
                },
              }
            : {}),
        },
        destination?.id,
      );
      await refresh();
      if (!destination) onCreated();
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : ui("Could not save destination"));
    } finally {
      setBusy(false);
    }
  }
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
    >
      <DialogContent
        chatProtected="Tracing destination credentials and delivery settings"
        className="max-h-[90dvh] overflow-y-auto sm:max-w-lg"
      >
        <DialogHeader>
          <DialogTitle>
            {ui(destination ? "Edit tracing destination" : "Add tracing destination")}
          </DialogTitle>
          <DialogDescription>
            {ui("Credentials stay encrypted and are never exposed to agents or Chat.")}
          </DialogDescription>
        </DialogHeader>
        <form className="space-y-4" onSubmit={(event) => void save(event)}>
          <div className="space-y-2">
            <Label htmlFor="trace-provider">{ui("Provider")}</Label>
            <Select
              value={config.provider}
              disabled={!!destination || busy}
              onValueChange={(provider) => {
                set({
                  provider: provider as TracingConfig["provider"],
                  endpoint: tracingProviders[provider as TracingConfig["provider"]].endpoint,
                  project_id: null,
                  project_name: null,
                  langsmith_agent_id: null,
                  langsmith_agent_environment: null,
                });
                setSecret("");
                setPublicKey("");
                setHeader("Authorization");
              }}
            >
              <SelectTrigger id="trace-provider" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {Object.entries(tracingProviders).map(([value, provider]) => (
                  <SelectItem key={value} value={value}>
                    {provider.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-2">
            <Label htmlFor="trace-name">{ui("Destination name")}</Label>
            <Input
              id="trace-name"
              required
              maxLength={128}
              value={config.name}
              onChange={(e) => set({ name: e.target.value })}
              disabled={busy}
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="trace-endpoint">{ui("Endpoint")}</Label>
            <Input
              id="trace-endpoint"
              type="url"
              required
              value={config.endpoint}
              onChange={(e) => set({ endpoint: e.target.value })}
              disabled={busy}
              placeholder="https://collector.example.com"
            />
            <p className="text-xs text-muted-foreground">
              {ui(
                "HTTPS only. Private collectors must be reachable through the deployment's approved network boundary.",
              )}
            </p>
          </div>
          {config.provider === "braintrust" && (
            <div className="space-y-2">
              <Label htmlFor="trace-project">{ui("Project ID")}</Label>
              <Input
                id="trace-project"
                required
                value={config.project_id ?? ""}
                onChange={(e) => set({ project_id: e.target.value || null })}
                disabled={busy}
              />
            </div>
          )}
          {config.provider === "langsmith" && (
            <div className="space-y-2">
              <label className="flex items-center gap-2 text-sm">
                <Checkbox
                  checked={config.langsmith_agent_id != null}
                  disabled={busy}
                  onCheckedChange={(v) =>
                    set({
                      project_name: v ? null : "",
                      langsmith_agent_id: v ? "" : null,
                      langsmith_agent_environment: v ? "production" : null,
                    })
                  }
                />
                {ui("Agent-based LangSmith workspace")}
              </label>
              {config.langsmith_agent_id != null ? (
                <>
                  <Label htmlFor="trace-langsmith-agent">{ui("LangSmith agent ID")}</Label>
                  <Input
                    id="trace-langsmith-agent"
                    required
                    maxLength={63}
                    pattern="[a-z]([a-z0-9-]*[a-z0-9])?"
                    value={config.langsmith_agent_id}
                    disabled={busy}
                    onChange={(e) => set({ langsmith_agent_id: e.target.value })}
                  />
                  <Label htmlFor="trace-langsmith-environment">{ui("LangSmith environment")}</Label>
                  <select
                    id="trace-langsmith-environment"
                    className="w-full rounded-md border p-2"
                    disabled={busy}
                    value={config.langsmith_agent_environment ?? "production"}
                    onChange={(e) => set({ langsmith_agent_environment: e.target.value })}
                  >
                    {["local", "development", "staging", "production"].map((v) => (
                      <option key={v} value={v}>
                        {v}
                      </option>
                    ))}
                  </select>
                </>
              ) : (
                <>
                  <Label htmlFor="trace-project-name">{ui("Project name")}</Label>
                  <Input
                    id="trace-project-name"
                    required
                    maxLength={256}
                    value={config.project_name ?? ""}
                    onChange={(e) => set({ project_name: e.target.value || null })}
                    disabled={busy}
                  />
                </>
              )}
            </div>
          )}
          {destination && !receiverChanged && (
            <label className="flex items-center gap-2 text-sm">
              <Checkbox
                checked={replaceAuth}
                onCheckedChange={(v) => setReplaceAuth(!!v)}
                disabled={busy}
              />
              {ui("Replace authentication")}
            </label>
          )}
          {needsSecret && (
            <>
              {config.provider === "otlp" && (
                <div className="space-y-2">
                  <Label htmlFor="trace-header">{ui("Authentication header")}</Label>
                  <Select value={header} onValueChange={setHeader}>
                    <SelectTrigger id="trace-header" className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {["Authorization", "X-API-Key", "Api-Key"].map((h) => (
                        <SelectItem key={h} value={h}>
                          {h}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
              )}
              {config.provider === "langfuse" && (
                <div className="space-y-2">
                  <Label htmlFor="trace-public-key">{ui("Public key")}</Label>
                  <Input
                    id="trace-public-key"
                    required
                    autoComplete="off"
                    value={publicKey}
                    onChange={(e) => setPublicKey(e.target.value)}
                    disabled={busy}
                    placeholder="pk-lf-…"
                  />
                </div>
              )}
              <div className="space-y-2">
                <Label htmlFor="trace-secret">
                  {ui(
                    config.provider === "langfuse"
                      ? "Secret key"
                      : config.provider === "otlp"
                        ? "Header value (optional)"
                        : "API key",
                  )}
                </Label>
                <Input
                  id="trace-secret"
                  type="password"
                  autoComplete="off"
                  required={config.provider !== "otlp"}
                  value={secret}
                  onChange={(e) => setSecret(e.target.value)}
                  disabled={busy}
                  placeholder={config.provider === "otlp" ? "Bearer …" : undefined}
                />
              </div>
            </>
          )}
          {destination && (
            <div className="space-y-3 border-t pt-4">
              <label className="flex items-start gap-2 text-sm">
                <Checkbox
                  checked={config.capture_content}
                  disabled={busy}
                  onCheckedChange={(v) =>
                    set({
                      capture_content: !!v,
                      capture_reasoning: !!v && config.capture_reasoning,
                    })
                  }
                />
                <span>
                  {ui("Include messages and tool content")}
                  <span className="mt-1 block text-xs text-muted-foreground">
                    {ui("Prompts, tool results, and detailed errors will leave Everruns.")}
                  </span>
                </span>
              </label>
              <label className="flex items-center gap-2 text-sm">
                <Checkbox
                  checked={config.capture_reasoning}
                  disabled={busy || !config.capture_content}
                  onCheckedChange={(v) => set({ capture_reasoning: !!v })}
                />
                {ui("Include reasoning content")}
              </label>
            </div>
          )}
          {!destination && (
            <p className="text-xs text-muted-foreground">
              {ui(
                "Starts with metadata only. Creating a destination does not assign it to agents.",
              )}
            </p>
          )}
          {error && (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          )}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={onClose} disabled={busy}>
              {ui("Cancel")}
            </Button>
            <Button type="submit" disabled={busy}>
              {ui(busy ? "Saving…" : destination ? "Save changes" : "Create destination")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
