"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { useEffect, useState } from "react";
import { Check } from "lucide-react";
import { useSearchParams } from "next/navigation";
import { usePageTitle } from "@/hooks";
import { ConnectErrorBanner } from "@/components/connections/connect-error-banner";
import { UserMcpServersPanel } from "@/components/connections/user-mcp-servers-panel";
import { stripConnectResultParams } from "@/lib/connect-error";

export default function McpServersSettingsPage() {
  const ui = useUiTranslations();
  usePageTitle(ui("MCP servers"), ui("Settings"));
  const searchParams = useSearchParams();
  const [success, setSuccess] = useState(() =>
    searchParams.get("connected") ? "Signed in." : null,
  );

  useEffect(() => {
    if (!success) return;
    stripConnectResultParams();
    const timer = setTimeout(() => setSuccess(null), 4000);
    return () => clearTimeout(timer);
  }, [success]);

  return (
    <div className="space-y-8">
      <ConnectErrorBanner />
      {success && (
        <div className="flex items-center gap-2 bg-success/10 p-3 text-sm text-success">
          <Check className="h-4 w-4" />
          {success}
        </div>
      )}
      <UserMcpServersPanel />
    </div>
  );
}
