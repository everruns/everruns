"use client";

import { useQuery } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { getSandboxProviderState } from "@/lib/api/sandboxes";
import { formatDate } from "@/lib/formatting";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";
import { useOrg } from "@/providers/org-provider";

export function SandboxProviderStatus({ sandboxId }: { sandboxId: string }) {
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  const { currentOrg, hasRole } = useOrg();
  const query = useQuery({
    queryKey: ["sandbox-fleet", "provider", currentOrg?.public_id, sandboxId],
    queryFn: () => getSandboxProviderState(sandboxId),
    enabled: false,
    retry: false,
  });
  if (!hasRole("admin")) return null;
  return (
    <section className="space-y-2">
      <Button
        size="sm"
        variant="outline"
        loading={query.isFetching}
        onClick={() => void query.refetch()}
      >
        {ui("Check provider status")}
      </Button>
      {query.data && (
        <p className="text-sm" role="status">
          <span className="font-medium">
            {query.data.exists ? query.data.state : ui("Not found at provider")}
          </span>
          <span className="pl-2 text-xs text-muted-foreground">
            {ui("Checked {value0}", { value0: formatDate(query.data.checked_at, displayLocale) })}
          </span>
        </p>
      )}
      {query.isError && (
        <p className="text-sm text-destructive" role="alert">
          {ui("Could not check the provider.")}
        </p>
      )}
    </section>
  );
}
