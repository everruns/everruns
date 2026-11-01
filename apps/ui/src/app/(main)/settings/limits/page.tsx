"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { Gauge } from "lucide-react";
import { BackgroundLimitsSettings } from "@/components/organizations/background-limits-settings";
import { Skeleton } from "@/components/ui/skeleton";
import { usePageTitle } from "@/hooks";
import { useOrganization } from "@/hooks/use-organizations";
import { useOrg } from "@/providers/org-provider";

export default function LimitsSettingsPage() {
  const ui = useUiTranslations();
  usePageTitle(ui("Limits"), ui("Settings"));
  const { hasRole } = useOrg();
  const { data: organization, isLoading, error } = useOrganization();
  if (isLoading) return <Skeleton className="h-48 w-full" />;
  if (error || !organization) return <p role="alert">{ui("Unable to load limits.")}</p>;
  return (
    <div className="space-y-6">
      <div className="flex items-center gap-3">
        <Gauge className="size-5" />
        <h1 className="text-xl font-semibold">{ui("Limits")}</h1>
      </div>
      {/* A new organization gets a fresh form; pending edits never cross tenants. */}
      <BackgroundLimitsSettings
        key={organization.id}
        organization={organization}
        canManage={hasRole("admin")}
      />
    </div>
  );
}
