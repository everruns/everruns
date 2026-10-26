"use client";

import { usePageTitle } from "@/hooks";
import { ConnectedClientsPanel } from "@/components/connections/connected-clients-panel";

export default function AiClientsPage() {
  usePageTitle("AI clients", "Settings");
  return <ConnectedClientsPanel />;
}
