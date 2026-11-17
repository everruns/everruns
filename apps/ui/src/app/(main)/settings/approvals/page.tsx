"use client";

import { ShieldCheck } from "lucide-react";
import { useUiTranslations } from "@/providers/locale-provider";
import { ToolApprovalPolicySettings } from "@/components/organizations/tool-approval-policy-settings";
import { Skeleton } from "@/components/ui/skeleton";
import { usePageTitle } from "@/hooks";
import { useOrganization } from "@/hooks/use-organizations";
import { useOrg } from "@/providers/org-provider";

export default function ApprovalsSettingsPage() {
  const ui = useUiTranslations();
  usePageTitle(ui("Approvals"), ui("Settings"));
  const { hasRole } = useOrg();
  const { data: organization, isLoading, error } = useOrganization();
  if (isLoading) return <Skeleton className="h-48 w-full" />;
  if (error || !organization) {
    return <p role="alert">{ui("Unable to load approval settings.")}</p>;
  }
  return (
    <div className="space-y-6">
      <div className="space-y-1">
        <div className="flex items-center gap-3">
          <ShieldCheck className="size-5" />
          <h1 className="text-xl font-semibold">{ui("Approvals")}</h1>
        </div>
        <p className="text-sm text-muted-foreground">
          {ui("How agents in this organization get a go-ahead before they use a tool.")}
        </p>
      </div>
      {/* A new organization gets a fresh form; pending edits never cross tenants. */}
      <ToolApprovalPolicySettings
        key={organization.id}
        organization={organization}
        canManage={hasRole("admin")}
      />
    </div>
  );
}
