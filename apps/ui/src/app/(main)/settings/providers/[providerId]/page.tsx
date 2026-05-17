"use client";

import { use, useEffect, useMemo, useState } from "react";
import Link from "next/link";
import { ArrowLeft, Boxes, Key, Save } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { PageBody, PageHeader, PageShell } from "@/components/layout";
import { ProviderIcon, getProviderLabel } from "@/components/providers/provider-icon";
import { ResourceNotFound } from "@/components/resource-not-found";
import { useLlmModels, useLlmProvider, useUpdateLlmProvider } from "@/hooks/use-llm-providers";
import { usePageTitle } from "@/hooks";
import { formatCountLabel } from "@/lib/formatting";

export default function ProviderDetailPage({
  params,
}: {
  params: Promise<{ providerId: string }>;
}) {
  const { providerId } = use(params);
  const { data: provider, isLoading } = useLlmProvider(providerId);
  const { data: models = [] } = useLlmModels();
  const updateProvider = useUpdateLlmProvider(providerId);
  const [name, setName] = useState("");
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  usePageTitle(provider ? provider.name : null, "Provider");

  useEffect(() => {
    if (provider) setName(provider.name);
  }, [provider]);

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
            <form onSubmit={handleSubmit} className="space-y-4 max-w-xl">
              <div className="space-y-2">
                <Label htmlFor="provider-name">Name</Label>
                <Input
                  id="provider-name"
                  value={name}
                  onChange={(event) => setName(event.target.value)}
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
              <Button
                type="submit"
                disabled={updateProvider.isPending || !trimmedName || !nameChanged}
              >
                <Save className="h-4 w-4 mr-2" />
                {updateProvider.isPending ? "Saving..." : "Save Changes"}
              </Button>
            </form>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Models</CardTitle>
          </CardHeader>
          <CardContent className="flex flex-wrap items-center gap-3 text-sm">
            <span className="text-muted-foreground">
              {formatCountLabel(modelCounts.total, "model")} available
            </span>
            <Badge variant="outline">{modelCounts.enabled} enabled</Badge>
            <Link href={`/models?provider=${encodeURIComponent(provider.id)}`}>
              <Button variant="outline" size="sm">
                View provider models
              </Button>
            </Link>
          </CardContent>
        </Card>
      </PageBody>
    </PageShell>
  );
}
