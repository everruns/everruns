"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { usePageTitle } from "@/hooks";
import { ConnectedClientsPanel } from "@/components/connections/connected-clients-panel";

export default function AiClientsPage() {
  const ui = useUiTranslations();
  usePageTitle(ui("AI clients"), ui("Settings"));
  return <ConnectedClientsPanel />;
}
