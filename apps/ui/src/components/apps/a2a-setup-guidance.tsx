"use client";

import { useCallback, useEffect, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/ui/copy-button";
import { A2aAgentCard, A2aAgentCardPreview } from "@/components/apps/a2a-agent-card-preview";
import { getInvocationSessionModeDisplayName } from "@/lib/app-channels";
import type { InvocationSessionMode } from "@/lib/api/types";
import { Bot, Globe, KeyRound, RefreshCw } from "lucide-react";

interface A2aSetupGuidanceProps {
  endpointUrl: string;
  agentCardUrl: string;
  apiKeyPrefix: string;
  sessionMode: InvocationSessionMode;
  message: string;
  appName: string;
  agentCardName?: string;
  agentCardDescription?: string;
  isPublished: boolean;
  channelEnabled: boolean;
  onConfigure?: () => void;
  onRotateKey?: () => void;
  isRotating?: boolean;
}

export function A2aSetupGuidance({
  endpointUrl,
  agentCardUrl,
  apiKeyPrefix,
  sessionMode,
  message,
  appName,
  agentCardName,
  agentCardDescription,
  isPublished,
  channelEnabled,
  onConfigure,
  onRotateKey,
  isRotating,
}: A2aSetupGuidanceProps) {
  const [agentCard, setAgentCard] = useState<A2aAgentCard | null>(null);
  const [agentCardError, setAgentCardError] = useState<string | null>(null);
  const [agentCardLoading, setAgentCardLoading] = useState(false);
  const canFetchAgentCard = isPublished && channelEnabled;

  const fetchAgentCard = useCallback(async () => {
    if (!canFetchAgentCard) return;
    setAgentCardLoading(true);
    setAgentCardError(null);
    try {
      const response = await fetch(agentCardUrl, {
        headers: { Accept: "application/json" },
        cache: "no-store",
      });
      if (!response.ok) {
        throw new Error(`HTTP ${response.status}`);
      }
      setAgentCard((await response.json()) as A2aAgentCard);
    } catch (error) {
      setAgentCard(null);
      setAgentCardError(error instanceof Error ? error.message : "Unable to fetch Agent Card");
    } finally {
      setAgentCardLoading(false);
    }
  }, [agentCardUrl, canFetchAgentCard]);

  useEffect(() => {
    if (!canFetchAgentCard) {
      setAgentCard(null);
      setAgentCardError(null);
      setAgentCardLoading(false);
      return;
    }
    fetchAgentCard();
  }, [canFetchAgentCard, fetchAgentCard]);

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-2">
        <Badge variant="default">A2A (Agent2Agent)</Badge>
        <span className="text-sm text-muted-foreground">
          {isPublished
            ? "Ready to accept authenticated A2A JSON-RPC requests."
            : "Publish the app to accept A2A requests."}
        </span>
      </div>

      <div>
        <p className="text-sm font-medium">JSON-RPC endpoint</p>
        <div className="mt-2 flex items-center gap-2 bg-muted p-3">
          <Globe className="h-4 w-4 shrink-0 text-muted-foreground" />
          <code className="flex-1 truncate text-sm">{endpointUrl}</code>
          <CopyButton value={endpointUrl} />
        </div>
        <p className="mt-1 text-xs text-muted-foreground">
          Send <code>POST</code> with a JSON-RPC 2.0 envelope. Only the <code>message/send</code>{" "}
          method is supported in this iteration.
        </p>
      </div>

      <div>
        <p className="text-sm font-medium">Agent Card</p>
        <div className="mt-2 flex items-center gap-2 bg-muted p-3">
          <Bot className="h-4 w-4 shrink-0 text-muted-foreground" />
          <code className="flex-1 truncate text-sm">{agentCardUrl}</code>
          <CopyButton value={agentCardUrl} />
        </div>
        <p className="mt-1 text-xs text-muted-foreground">
          Public discovery document for A2A clients. Returns 404 unless the app is published and the
          channel is enabled.
        </p>
      </div>

      <div className="grid gap-4 md:grid-cols-2">
        <div>
          <p className="text-sm font-medium">Authentication</p>
          <div className="mt-2 flex items-center gap-2 text-sm text-muted-foreground">
            <KeyRound className="h-4 w-4 shrink-0" />
            <span>
              <code>Authorization: Bearer &lt;api_key&gt;</code>
            </span>
          </div>
          <div className="mt-2 flex items-center gap-2 bg-muted p-2">
            <code className="flex-1 truncate text-xs font-mono">
              {apiKeyPrefix || "(no key configured)"}
            </code>
          </div>
          <p className="mt-1 text-xs text-muted-foreground">
            The plaintext key is shown once at create / rotate time. Only the prefix is stored for
            display; the full value is hashed at rest.
          </p>
        </div>
        <div>
          <p className="text-sm font-medium">Session Mode</p>
          <p className="text-sm text-muted-foreground">
            {getInvocationSessionModeDisplayName(sessionMode)}
          </p>
        </div>
      </div>

      {(agentCardName || agentCardDescription) && (
        <div className="grid gap-4 md:grid-cols-2">
          {agentCardName && (
            <div>
              <p className="text-sm font-medium">Card name</p>
              <p className="text-sm text-muted-foreground">{agentCardName}</p>
            </div>
          )}
          {agentCardDescription && (
            <div>
              <p className="text-sm font-medium">Card description</p>
              <p className="text-sm text-muted-foreground">{agentCardDescription}</p>
            </div>
          )}
        </div>
      )}

      <div>
        <div className="mb-2 flex items-center justify-between gap-3">
          <p className="text-sm font-medium">Discovered Agent Card</p>
          {canFetchAgentCard && (
            <Button
              type="button"
              size="sm"
              variant="outline"
              onClick={fetchAgentCard}
              disabled={agentCardLoading}
            >
              <RefreshCw className="mr-1 h-3 w-3" />
              {agentCardLoading ? "Refreshing" : "Refresh"}
            </Button>
          )}
        </div>
        {!canFetchAgentCard ? (
          <div className="rounded-md border p-3 text-sm text-muted-foreground">
            Publish the app and enable this channel to fetch the public Agent Card. Until then,
            discovery returns 404.
          </div>
        ) : agentCard ? (
          <A2aAgentCardPreview card={agentCard} appName={appName} sessionMode={sessionMode} />
        ) : (
          <div className="rounded-md border p-3 text-sm text-muted-foreground">
            {agentCardLoading ? "Loading Agent Card..." : "Could not load Agent Card."}
            {agentCardError && <span className="ml-1">({agentCardError})</span>}
            <div className="mt-2 flex items-center gap-2 bg-muted p-2">
              <code className="flex-1 truncate text-xs">{agentCardUrl}</code>
              <CopyButton value={agentCardUrl} />
            </div>
          </div>
        )}
      </div>

      <div>
        <p className="text-sm font-medium">Invocation Message</p>
        <div className="mt-2 rounded-md border p-3 text-sm text-muted-foreground">
          <code className="whitespace-pre-wrap break-words">{message}</code>
        </div>
        <p className="mt-1 text-xs text-muted-foreground">
          Templates can reference <code>{"{{a2a.text}}"}</code>, <code>{"{{payload}}"}</code>, and
          app metadata.
        </p>
      </div>

      <div className="flex flex-wrap gap-2 pt-1">
        {onConfigure && (
          <Button size="sm" variant="outline" onClick={onConfigure}>
            Configure
          </Button>
        )}
        {onRotateKey && (
          <Button size="sm" variant="outline" onClick={onRotateKey} disabled={isRotating}>
            <RefreshCw className="mr-1 h-3 w-3" />
            {isRotating ? "Rotating..." : "Rotate API key"}
          </Button>
        )}
      </div>
    </div>
  );
}
