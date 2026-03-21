"use client";

/**
 * Inline connection setup card rendered when a tool call requires a user connection.
 *
 * The worker emits a synthetic `setup_connection` client-side tool call when a
 * server-side tool (e.g. daytona_create_sandbox) discovers no connection is
 * configured. This component renders an inline card with a "Connect" button;
 * clicking it opens the shared ApiKeyDialog. After the user saves (or cancels),
 * a tool result is submitted via the API and the workflow resumes.
 */

import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Check, LinkIcon, X } from "lucide-react";
import { useConnectionProviders } from "@/hooks/use-user-connections";
import { ApiKeyDialog } from "@/components/connections/api-key-dialog";
import { ProviderIcon } from "@/components/connections/provider-icon";
import { submitToolResults } from "@/lib/api/sessions";
import { getBackendUrl } from "@/lib/api/client";
import { cn } from "@/lib/utils";
import type { ToolCompletedData } from "@/lib/api/types";

interface SetupConnectionToolCallProps {
  sessionId: string;
  toolCallId: string;
  /** Provider id from the tool call arguments (e.g. "daytona") */
  provider: string;
  /** Existing tool results map — if a result already exists, show completed state */
  toolResultsMap: Map<string, ToolCompletedData>;
}

export function SetupConnectionToolCall({
  sessionId,
  toolCallId,
  provider,
  toolResultsMap,
}: SetupConnectionToolCallProps) {
  const { data: providers = [] } = useConnectionProviders();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [status, setStatus] = useState<"idle" | "connected" | "cancelled" | "submitting">("idle");

  const providerInfo = providers.find((p) => p.provider_id === provider);
  const displayName = providerInfo?.display_name ?? provider;
  const icon = providerInfo?.icon ?? "link";
  // Only MCP OAuth providers use the popup authorize flow; GitHub/other OAuth
  // providers have their own dedicated connection flow.
  const isOAuth = providerInfo?.connection_type === "oauth" && provider.startsWith("mcp_oauth_");

  // If we already have a tool result for this call, show completed state
  const existingResult = toolResultsMap.get(toolCallId);
  const isCompleted = existingResult != null || status === "connected" || status === "cancelled";

  const handleConnected = async () => {
    setStatus("submitting");
    try {
      await submitToolResults(sessionId, [
        {
          tool_call_id: toolCallId,
          result: { connected: true, provider },
        },
      ]);
      setStatus("connected");
    } catch {
      // Even if submit fails, the dialog already saved the connection
      setStatus("connected");
    }
  };

  const handleOAuthConnect = () => {
    const returnTo = `${window.location.pathname}${window.location.search}`;
    const popup = window.open(
      `${getBackendUrl()}/v1/user/connections/${encodeURIComponent(provider)}/authorize?mode=session&session_id=${encodeURIComponent(sessionId)}&popup=true&return_to=${encodeURIComponent(returnTo)}`,
      "everruns-mcp-auth",
      "popup,width=720,height=820",
    );
    if (!popup) return;
    let timeoutId: number | undefined;
    const listener = (event: MessageEvent) => {
      if (event.origin !== window.location.origin) return;
      if (event.data?.type !== "everruns:connection-complete") return;
      if (event.data?.provider !== provider) return;
      window.removeEventListener("message", listener);
      if (timeoutId !== undefined) window.clearTimeout(timeoutId);
      void handleConnected();
    };
    window.addEventListener("message", listener);
    // Auto-cleanup listener after 2 minutes to prevent leaks if popup is
    // closed without completing the flow.
    timeoutId = window.setTimeout(
      () => {
        window.removeEventListener("message", listener);
      },
      2 * 60 * 1000,
    );
  };

  const handleCancelled = async () => {
    setStatus("submitting");
    try {
      await submitToolResults(sessionId, [
        {
          tool_call_id: toolCallId,
          error: "Connection setup cancelled by user",
        },
      ]);
    } catch {
      // best-effort
    }
    setStatus("cancelled");
  };

  if (isCompleted) {
    const wasSuccess = status === "connected" || existingResult?.success;
    return (
      <div
        className={cn(
          "flex items-center gap-2 px-3 py-1.5 text-sm",
          wasSuccess ? "text-green-700 dark:text-green-400" : "text-muted-foreground",
        )}
      >
        {wasSuccess ? <Check className="h-4 w-4" /> : <X className="h-4 w-4" />}
        <ProviderIcon iconName={icon} className="h-4 w-4" />
        {wasSuccess ? `${displayName} connected` : `${displayName} connection cancelled`}
      </div>
    );
  }

  return (
    <>
      <div className="flex items-center gap-3 rounded-lg border border-border bg-muted/50 px-4 py-3">
        <ProviderIcon iconName={icon} className="h-5 w-5 text-foreground" />
        <div className="flex-1 min-w-0">
          <p className="text-sm font-medium text-foreground">{displayName} connection required</p>
          <p className="text-xs text-muted-foreground">
            Connect your {displayName} account to continue
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button
            size="sm"
            variant="ghost"
            onClick={handleCancelled}
            disabled={status === "submitting"}
            className="text-muted-foreground"
          >
            Skip
          </Button>
          <Button
            size="sm"
            onClick={() => (isOAuth ? handleOAuthConnect() : setDialogOpen(true))}
            disabled={status === "submitting"}
          >
            <LinkIcon className="h-3.5 w-3.5 mr-1" />
            Connect
          </Button>
        </div>
      </div>

      {!isOAuth && (
        <ApiKeyDialog
          provider={providerInfo ?? null}
          open={dialogOpen}
          onOpenChange={(open) => {
            setDialogOpen(open);
            // If dialog was closed without connecting (no onConnected callback fired)
            // we don't submit cancellation — user might reopen
          }}
          onConnected={handleConnected}
        />
      )}
    </>
  );
}
