"use client";

import { Suspense } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { Receipt } from "lucide-react";

import { CostsOverview } from "@/components/costs/costs-overview";
import { PriceList } from "@/components/costs/price-list";
import { UsagePolicies } from "@/components/costs/usage-policies";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { usePageTitle } from "@/hooks";
import { useOrg } from "@/providers/org-provider";
import { useUiTranslations } from "@/providers/locale-provider";

const TABS = ["overview", "prices", "policies"] as const;
type CostsTab = (typeof TABS)[number];

export default function CostsSettingsPage() {
  // The tab lives in the URL (`?tab=policies` is where notifications link).
  return (
    <Suspense>
      <CostsSettings />
    </Suspense>
  );
}

function CostsSettings() {
  const ui = useUiTranslations();
  usePageTitle(ui("Costs"), ui("Settings"));
  const { hasRole } = useOrg();
  const canManage = hasRole("admin");
  const router = useRouter();
  const params = useSearchParams();
  const requested = params.get("tab");
  const tab: CostsTab = TABS.includes(requested as CostsTab) ? (requested as CostsTab) : "overview";

  return (
    <div className="space-y-6">
      <div className="space-y-1">
        <div className="flex items-center gap-3">
          <Receipt className="size-5" />
          <h1 className="text-xl font-semibold">{ui("Costs")}</h1>
        </div>
        <p className="text-sm text-muted-foreground">
          {ui(
            "What your agents cost this organization, from your price list. Every amount says where its number came from.",
          )}
        </p>
      </div>
      <Tabs value={tab} onValueChange={(value) => router.replace(`/settings/costs?tab=${value}`)}>
        <TabsList>
          <TabsTrigger value="overview">{ui("Overview")}</TabsTrigger>
          <TabsTrigger value="prices">{ui("Price list")}</TabsTrigger>
          <TabsTrigger value="policies">{ui("Usage policies")}</TabsTrigger>
        </TabsList>
        <TabsContent value="overview" className="mt-4">
          <CostsOverview />
        </TabsContent>
        <TabsContent value="prices" className="mt-4">
          <PriceList canManage={canManage} />
        </TabsContent>
        <TabsContent value="policies" className="mt-4">
          <UsagePolicies canManage={canManage} />
        </TabsContent>
      </Tabs>
    </div>
  );
}
