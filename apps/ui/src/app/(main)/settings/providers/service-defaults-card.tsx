"use client";

import { Card } from "@/components/ui/card";
import { useOrganization, useUpdateOrganization } from "@/hooks/use-organizations";
import type { Provider } from "@/lib/api/types";

/**
 * Org-level default provider per service (EVE-569). Tier 2 of service-bound
 * resolution: after an explicit per-consumer binding and before the
 * single-active-provider fallback, an org may pin a default provider for a
 * given service. Chat is intentionally excluded — chat uses `default_model_id`.
 */
const SERVICES: { key: string; label: string; description: string }[] = [
  { key: "realtime", label: "Realtime (voice)", description: "Voice realtime sessions" },
  { key: "embeddings", label: "Embeddings", description: "Text embeddings" },
  { key: "images", label: "Images", description: "Image generation" },
  { key: "rerank", label: "Rerank", description: "Search-result reranking" },
];

export function ServiceDefaultsCard({ providers }: { providers: Provider[] }) {
  const { data: org } = useOrganization();
  const updateOrg = useUpdateOrganization();

  const defaults = org?.default_provider_per_service ?? {};

  const handleChange = async (service: string, providerId: string) => {
    const next: Record<string, string> = { ...defaults };
    if (providerId) {
      next[service] = providerId;
    } else {
      delete next[service];
    }
    await updateOrg.mutateAsync({ default_provider_per_service: next });
  };

  return (
    <section>
      <div className="mb-4">
        <h2 className="text-xl font-semibold">Service Defaults</h2>
        <p className="text-sm text-muted-foreground">
          Pin a default provider per service. Used when a request does not name a provider
          explicitly. A provider whose driver does not support the service is rejected when that
          service runs.
        </p>
      </div>

      <Card className="p-6 space-y-4">
        {SERVICES.map((service) => (
          <div key={service.key} className="flex items-center justify-between gap-4">
            <div>
              <div className="text-sm font-medium">{service.label}</div>
              <div className="text-xs text-muted-foreground">{service.description}</div>
            </div>
            <select
              aria-label={`Default provider for ${service.label}`}
              className="border rounded-md px-3 py-2 text-sm bg-background min-w-56"
              value={defaults[service.key] ?? ""}
              disabled={updateOrg.isPending}
              onChange={(event) => handleChange(service.key, event.target.value)}
            >
              <option value="">No default</option>
              {providers.map((provider) => (
                <option key={provider.id} value={provider.id}>
                  {provider.name}
                </option>
              ))}
            </select>
          </div>
        ))}
      </Card>
    </section>
  );
}
