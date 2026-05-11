import { Badge } from "@/components/ui/badge";
import { getInvocationSessionModeDisplayName } from "@/lib/app-channels";
import type { InvocationSessionMode } from "@/lib/api/types";
import { Bot, FileJson } from "lucide-react";

interface A2aAgentCardPreviewProps {
  appName: string;
  appDescription?: string | null;
  endpointUrl?: string;
  agentCardName?: string | null;
  agentCardDescription?: string | null;
  sessionMode: InvocationSessionMode;
}

export const A2A_AGENT_CARD_PROTOCOL_VERSION = "0.3.0";
export const A2A_AGENT_CARD_VERSION = "0.1";

export function buildA2aAgentCardPreview({
  appName,
  appDescription,
  endpointUrl,
  agentCardName,
  agentCardDescription,
  sessionMode,
}: A2aAgentCardPreviewProps) {
  const description = agentCardDescription?.trim() || appDescription?.trim() || "";

  return {
    name: agentCardName?.trim() || appName,
    description,
    url: endpointUrl || "(generated after save)",
    protocolVersion: A2A_AGENT_CARD_PROTOCOL_VERSION,
    version: A2A_AGENT_CARD_VERSION,
    preferredTransport: "JSONRPC",
    capabilities: {
      streaming: sessionMode === "session_per_invocation",
      pushNotifications: false,
      stateTransitionHistory: false,
    },
    defaultInputModes: ["text/plain"],
    defaultOutputModes: ["text/plain"],
    skills: [
      {
        id: "default",
        name: appName,
        description,
        tags: ["everruns", "a2a"],
      },
    ],
    securitySchemes: {
      apiKey: { type: "http", scheme: "bearer" },
    },
    security: [{ apiKey: [] }],
  };
}

export function A2aAgentCardPreview(props: A2aAgentCardPreviewProps) {
  const card = buildA2aAgentCardPreview(props);
  const previewJson = JSON.stringify(card, null, 2);

  return (
    <div className="rounded-md border bg-muted/20 p-3">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex min-w-0 items-start gap-2">
          <Bot className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
          <div className="min-w-0">
            <p className="text-sm font-medium">Agent Card preview</p>
            <p className="truncate text-xs text-muted-foreground">
              {card.name} - {card.preferredTransport}
            </p>
          </div>
        </div>
        <Badge variant={card.capabilities.streaming ? "default" : "secondary"}>
          {card.capabilities.streaming ? "Streaming" : "Send only"}
        </Badge>
      </div>

      <div className="mt-3 grid gap-3 text-sm md:grid-cols-2">
        <div>
          <p className="font-medium">Session mode</p>
          <p className="text-muted-foreground">
            {getInvocationSessionModeDisplayName(props.sessionMode)}
          </p>
        </div>
        <div>
          <p className="font-medium">Authentication</p>
          <p className="text-muted-foreground">Bearer API key</p>
        </div>
      </div>

      <div className="mt-3 flex items-center gap-2 text-xs font-medium text-muted-foreground">
        <FileJson className="h-3.5 w-3.5" />
        JSON
      </div>
      <pre
        aria-label="Agent Card JSON preview"
        className="mt-2 max-h-72 overflow-auto rounded-md bg-background p-3 text-xs leading-relaxed"
      >
        <code>{previewJson}</code>
      </pre>
    </div>
  );
}
