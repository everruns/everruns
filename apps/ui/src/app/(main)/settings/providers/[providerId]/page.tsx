"use client";

import { use, useEffect, useMemo, useState } from "react";
import Link from "next/link";
import { ArrowLeft, Boxes, Key, Save } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Skeleton } from "@/components/ui/skeleton";
import { PageBody, PageHeader, PageShell } from "@/components/layout";
import { ProviderIcon, getProviderLabel } from "@/components/providers/provider-icon";
import { ResourceNotFound } from "@/components/resource-not-found";
import { useModels, useProvider, useUpdateProvider } from "@/hooks/use-providers";
import { usePageTitle } from "@/hooks";
import { formatCountLabel } from "@/lib/formatting";
import type { Provider } from "@/lib/api/types";

export default function ProviderDetailPage({
  params,
}: {
  params: Promise<{ providerId: string }>;
}) {
  const { providerId } = use(params);
  const { data: provider, isLoading } = useProvider(providerId);
  const { data: models = [], isLoading: modelsLoading } = useModels();
  const updateProvider = useUpdateProvider(providerId);
  const [name, setName] = useState("");
  const [nameProviderId, setNameProviderId] = useState<string | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  usePageTitle(provider ? provider.name : null, "Provider");

  useEffect(() => {
    if (provider && nameProviderId !== provider.id) {
      setName(provider.name);
      setNameProviderId(provider.id);
    }
  }, [provider, nameProviderId]);

  const modelCounts = useMemo(() => {
    return models.reduce(
      (counts, model) => {
        if (model.provider_id !== providerId) return counts;
        counts.total += 1;
        if (model.enabled) counts.enabled += 1;
        return counts;
      },
      { total: 0, enabled: 0 },
    );
  }, [models, providerId]);

  const trimmedName = name.trim();
  const nameChanged = !!provider && trimmedName !== provider.name;

  const handleSubmit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!provider || !trimmedName || !nameChanged) return;
    setErrorMessage(null);
    try {
      await updateProvider.mutateAsync({ name: trimmedName });
    } catch {
      setErrorMessage("Failed to save provider.");
    }
  };

  if (isLoading) {
    return (
      <PageShell>
        <Skeleton className="h-5 w-40 mb-6" />
        <Skeleton className="h-8 w-1/3 mb-4" />
        <Skeleton className="h-52 w-full" />
      </PageShell>
    );
  }

  if (!provider) {
    return (
      <ResourceNotFound
        title="Provider not found"
        description="This provider may have been deleted, moved to another organization, or the URL may be wrong."
        backHref="/settings/providers"
        backLabel="Back to providers"
        resourceId={providerId}
      />
    );
  }

  return (
    <PageShell>
      <Link
        href="/settings/providers"
        className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-6"
      >
        <ArrowLeft className="w-4 h-4 mr-2" />
        Back to Providers
      </Link>

      <PageHeader
        title={
          <>
            <ProviderIcon providerType={provider.provider_type} size="md" />
            {provider.name}
            {provider.managed && (
              <Badge variant="outline" className="bg-blue-100 text-blue-800" title="Managed by the host">
                Managed
              </Badge>
            )}
            <Badge
              variant="outline"
              className={
                provider.status === "active"
                  ? "bg-green-100 text-green-800"
                  : "bg-gray-100 text-gray-800"
              }
            >
              {provider.status}
            </Badge>
          </>
        }
        description={getProviderLabel(provider.provider_type)}
        actions={
          <Link href={`/models?provider=${encodeURIComponent(provider.id)}`}>
            <Button variant="outline">
              <Boxes className="h-4 w-4 mr-2" />
              View Models
            </Button>
          </Link>
        }
      />

      <PageBody>
        <Card>
          <CardHeader>
            <CardTitle>Provider Settings</CardTitle>
          </CardHeader>
          <CardContent>
            {provider.managed && (
              <p className="mb-4 rounded-md bg-muted px-3 py-2 text-sm text-muted-foreground">
                This provider is managed by the host and is read-only. Its credentials and
                configuration cannot be changed here.
              </p>
            )}
            <form onSubmit={handleSubmit} className="space-y-4 max-w-xl">
              <div className="space-y-2">
                <Label htmlFor="provider-name">Name</Label>
                <Input
                  id="provider-name"
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                  disabled={provider.managed}
                  required
                />
              </div>
              {provider.base_url && (
                <div className="space-y-2">
                  <Label>Base URL</Label>
                  <p className="break-all text-sm text-muted-foreground">{provider.base_url}</p>
                </div>
              )}
              <div className="flex items-center gap-2 text-sm text-muted-foreground">
                <Key className="h-4 w-4" />
                API Key: {provider.api_key_set ? "Configured" : "Not set"}
              </div>
              {errorMessage && <p className="text-sm text-destructive">{errorMessage}</p>}
              {!provider.managed && (
                <Button
                  type="submit"
                  disabled={updateProvider.isPending || !trimmedName || !nameChanged}
                >
                  <Save className="h-4 w-4 mr-2" />
                  {updateProvider.isPending ? "Saving..." : "Save Changes"}
                </Button>
              )}
            </form>
          </CardContent>
        </Card>

        {/* Trace links are part of the host-managed config for managed providers. */}
        {!provider.managed && <ProviderTraceCard provider={provider} providerId={providerId} />}

        <Card>
          <CardHeader>
            <CardTitle>Models</CardTitle>
          </CardHeader>
          <CardContent className="flex flex-wrap items-center gap-3 text-sm">
            {modelsLoading ? (
              <>
                <Skeleton className="h-4 w-32" />
                <Skeleton className="h-5 w-20" />
              </>
            ) : (
              <>
                <span className="text-muted-foreground">
                  {formatCountLabel(modelCounts.total, "model")} available
                </span>
                <Badge variant="outline">{modelCounts.enabled} enabled</Badge>
                <Link href={`/models?provider=${encodeURIComponent(provider.id)}`}>
                  <Button variant="outline" size="sm">
                    View provider models
                  </Button>
                </Link>
              </>
            )}
          </CardContent>
        </Card>
      </PageBody>
    </PageShell>
  );
}

/**
 * Configures whether session chats show deep links to this provider's trace/logs
 * dashboard, and (optionally) overrides the URL templates. The backend resolves
 * driver defaults overlaid with these overrides; `enabled` defaults off because
 * providers retain trace content only when logging is turned on for the account.
 */
function ProviderTraceCard({ provider, providerId }: { provider: Provider; providerId: string }) {
  const updateProvider = useUpdateProvider(providerId);
  const [enabled, setEnabled] = useState(provider.trace?.enabled ?? false);
  const [generationTemplate, setGenerationTemplate] = useState(
    provider.trace?.generation_url_template ?? "",
  );
  const [sessionTemplate, setSessionTemplate] = useState(
    provider.trace?.session_url_template ?? "",
  );
  const [seededId, setSeededId] = useState<string | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  // Re-seed the form once the loaded provider (or a different one) arrives.
  useEffect(() => {
    if (seededId !== provider.id) {
      setEnabled(provider.trace?.enabled ?? false);
      setGenerationTemplate(provider.trace?.generation_url_template ?? "");
      setSessionTemplate(provider.trace?.session_url_template ?? "");
      setSeededId(provider.id);
    }
  }, [provider, seededId]);

  const handleSave = async (event: React.FormEvent) => {
    event.preventDefault();
    setErrorMessage(null);
    setSaved(false);
    try {
      await updateProvider.mutateAsync({
        trace: {
          enabled,
          generation_url_template: generationTemplate.trim() || undefined,
          session_url_template: sessionTemplate.trim() || undefined,
        },
      });
      setSaved(true);
    } catch {
      setErrorMessage("Failed to save trace settings.");
    }
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle>Trace Links</CardTitle>
      </CardHeader>
      <CardContent>
        <form onSubmit={handleSave} className="space-y-4 max-w-xl">
          <p className="text-sm text-muted-foreground">
            Show links from session chats to this provider&apos;s trace/logs dashboard. Enable this
            only after turning on request logging for the account — most providers retain
            prompt/completion content only when logging is enabled. Templates support the{" "}
            <code>{"{response_id}"}</code>, <code>{"{session_id}"}</code>,{" "}
            <code>{"{turn_id}"}</code> and <code>{"{model}"}</code> placeholders.
          </p>
          <div className="flex items-center justify-between gap-4">
            <Label htmlFor="trace-enabled">Enable trace links</Label>
            <Switch id="trace-enabled" checked={enabled} onCheckedChange={setEnabled} />
          </div>
          <div className="space-y-2">
            <Label htmlFor="trace-generation">Generation URL template</Label>
            <Input
              id="trace-generation"
              value={generationTemplate}
              onChange={(event) => setGenerationTemplate(event.target.value)}
              placeholder="https://openrouter.ai/logs?id={response_id}"
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="trace-session">Session URL template</Label>
            <Input
              id="trace-session"
              value={sessionTemplate}
              onChange={(event) => setSessionTemplate(event.target.value)}
              placeholder="https://openrouter.ai/logs"
            />
          </div>
          {errorMessage && <p className="text-sm text-destructive">{errorMessage}</p>}
          {saved && !errorMessage && (
            <p className="text-sm text-muted-foreground">Trace settings saved.</p>
          )}
          <Button type="submit" disabled={updateProvider.isPending}>
            <Save className="h-4 w-4 mr-2" />
            {updateProvider.isPending ? "Saving..." : "Save Trace Settings"}
          </Button>
        </form>
      </CardContent>
    </Card>
  );
}
