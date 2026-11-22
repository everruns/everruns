"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { Gauge } from "lucide-react";
import { BackgroundLimitsSettings } from "@/components/organizations/background-limits-settings";
import { MonthlyCreationLimitSettings } from "@/components/organizations/monthly-creation-limit-settings";
import { SelfHealingSettings } from "@/components/organizations/self-healing-settings";
import { Skeleton } from "@/components/ui/skeleton";
import { usePageTitle } from "@/hooks";
import { useOrganization } from "@/hooks/use-organizations";
import { useOrg } from "@/providers/org-provider";

export default function LimitsSettingsPage() {
  const ui = useUiTranslations();
  usePageTitle(ui("Limits"), ui("Settings"));
  const { hasRole } = useOrg();
  const canManage = hasRole("admin");
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
      {organization.monthly_session_limit !== undefined && (
        <MonthlyCreationLimitSettings
          key={`${organization.id}-session-limit`}
          field="monthly_session_limit"
          limit={organization.monthly_session_limit}
          canManage={canManage}
          copy={{
            title: ui("Session creation"),
            label: ui("Monthly session limit"),
            hint: ui(
              "The limit resets on the first day of each month at midnight UTC. Archived and deleted sessions still count for the month they were created.",
            ),
            saveError: ui("Could not save the session limit."),
          }}
        />
      )}
      {organization.monthly_agent_limit !== undefined && (
        <MonthlyCreationLimitSettings
          key={`${organization.id}-agent-limit`}
          field="monthly_agent_limit"
          limit={organization.monthly_agent_limit}
          canManage={canManage}
          copy={{
            title: ui("Agent creation"),
            label: ui("Monthly agent limit"),
            hint: ui(
              "The limit resets on the first day of each month at midnight UTC. Archived and deleted agents still count for the month they were created. Built-in agents never count.",
            ),
            saveError: ui("Could not save the agent limit."),
          }}
        />
      )}
      <BackgroundLimitsSettings
        key={organization.id}
        organization={organization}
        canManage={canManage}
      />
      <SelfHealingSettings
        key={`${organization.id}-self-healing`}
        organization={organization}
        canManage={canManage}
      />
    </div>
  );
}
