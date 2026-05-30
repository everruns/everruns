"use client";

import { AlertCircle, FlaskConical, Loader2 } from "lucide-react";
import { Card } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import {
  useOrgFeatureFlagSettings,
  useUpdateOrgFeatureFlags,
} from "@/hooks/use-org-feature-flags";
import { usePageTitle } from "@/hooks";
import { useOrg } from "@/providers/org-provider";
import type { OrgFeatureFlagSetting } from "@/lib/api/types";

export default function FeaturesSettingsPage() {
  usePageTitle("Features", "Settings");
  const { currentOrg } = useOrg();
  const canManage = currentOrg?.role === "owner" || currentOrg?.role === "admin";
  const { data, isLoading, error } = useOrgFeatureFlagSettings();
  const updateFlags = useUpdateOrgFeatureFlags();

  const availableFlags = data?.flags.filter((f) => f.system_enabled) ?? [];
  const unavailableCount = data?.flags.filter((f) => !f.system_enabled).length ?? 0;

  const handleToggle = (flag: OrgFeatureFlagSetting, enabled: boolean) => {
    if (!canManage) return;
    updateFlags.mutate({ [flag.name]: enabled });
  };

  return (
    <div className="mx-auto max-w-3xl space-y-6 p-6">
      <div>
        <h2 className="text-lg font-semibold">Features</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          Choose which experimental and optional capabilities are enabled for{" "}
          <span className="font-medium text-foreground">{currentOrg?.name ?? "this organization"}</span>
          . Flags must be available on this deployment before your organization can opt in.
        </p>
      </div>

      {!canManage && (
        <Card className="flex items-start gap-3 border-amber-500/30 bg-amber-500/5 p-4 text-sm">
          <AlertCircle className="mt-0.5 h-4 w-4 shrink-0 text-amber-600" />
          <p>Only organization owners and admins can change feature settings.</p>
        </Card>
      )}

      {error && (
        <Card className="border-destructive/50 p-4 text-sm text-destructive">
          Failed to load feature settings. Try refreshing the page.
        </Card>
      )}

      {isLoading && (
        <div className="space-y-3">
          <Skeleton className="h-20 w-full" />
          <Skeleton className="h-20 w-full" />
          <Skeleton className="h-20 w-full" />
        </div>
      )}

      {!isLoading && availableFlags.length === 0 && (
        <Card className="p-6 text-sm text-muted-foreground">
          No optional features are available to enable on this deployment.
          {unavailableCount > 0 &&
            ` (${unavailableCount} feature${unavailableCount === 1 ? "" : "s"} require operator configuration.)`}
        </Card>
      )}

      <div className="space-y-3">
        {availableFlags.map((flag) => (
          <Card key={flag.name} className="flex items-start justify-between gap-4 p-4">
            <div className="min-w-0 space-y-1">
              <div className="flex flex-wrap items-center gap-2">
                <Label htmlFor={`flag-${flag.name}`} className="text-base font-medium">
                  {flag.label}
                </Label>
                {flag.experimental && (
                  <span className="inline-flex items-center gap-1 text-xs text-amber-600">
                    <FlaskConical className="h-3.5 w-3.5" />
                    Experimental
                  </span>
                )}
              </div>
              <p className="text-sm text-muted-foreground">{flag.description}</p>
            </div>
            <div className="flex shrink-0 items-center gap-2 pt-1">
              {updateFlags.isPending && updateFlags.variables && flag.name in updateFlags.variables && (
                <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
              )}
              <Switch
                id={`flag-${flag.name}`}
                checked={flag.org_enabled}
                disabled={!canManage || updateFlags.isPending}
                onCheckedChange={(checked) => handleToggle(flag, checked)}
                aria-label={`Enable ${flag.label}`}
              />
            </div>
          </Card>
        ))}
      </div>
    </div>
  );
}
