import { Badge } from "@/components/ui/badge";
import { getInvocationSessionModeDisplayName } from "@/lib/app-channels";
import type { InvocationSessionMode } from "@/lib/api/types";
import { Bot, FileJson } from "lucide-react";
import { EntityIdentity } from "@/components/ui/entity-identity";

type JsonObject = Record<string, unknown>;

export interface A2aAgentCard {
  [key: string]: unknown;
  name: string;
  description?: string;
  url: string;
  protocolVersion: string;
  version?: string;
  preferredTransport: string;
  capabilities: Record<string, unknown>;
  defaultInputModes?: string[];
  defaultOutputModes?: string[];
  skills?: Array<{
    id?: string;
    name?: string;
    description?: string;
    tags?: string[];
  }>;
  securitySchemes?: JsonObject;
  security?: unknown;
}

interface A2aAgentCardPreviewProps {
  card?: A2aAgentCard;
  appName: string;
  appDescription?: string | null;
  endpointUrl?: string;
  agentCardName?: string | null;
  agentCardDescription?: string | null;
  sessionMode: InvocationSessionMode;
}

export const A2A_AGENT_CARD_PROTOCOL_VERSION = "0.3.0";
export const A2A_AGENT_CARD_VERSION = "0.1";

function redactSecretLikeValues(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(redactSecretLikeValues);
  }
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value as JsonObject)
        .filter(([key]) => key !== "api_key" && key !== "api_key_hash" && key !== "apiKeyHash")
        .map(([key, nested]) => [key, redactSecretLikeValues(nested)]),
    );
  }
  if (typeof value === "string") {
    if (/^evra2a_[a-f0-9]{16,}$/i.test(value) || /^[a-f0-9]{64}$/i.test(value)) {
      return "[redacted]";
    }
  }
  return value;
}

export function sanitizeA2aAgentCardForDisplay(card: A2aAgentCard): A2aAgentCard {
  return redactSecretLikeValues(card) as A2aAgentCard;
}

export function buildA2aAgentCardPreview({
  appName,
  appDescription,
  endpointUrl,
  agentCardName,
  agentCardDescription,
  sessionMode,
}: Omit<A2aAgentCardPreviewProps, "card">): A2aAgentCard {
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
  const card = sanitizeA2aAgentCardForDisplay(props.card ?? buildA2aAgentCardPreview(props));
  const previewJson = JSON.stringify(card, null, 2);
  const capabilityEntries = Object.entries(card.capabilities ?? {});
  const securitySchemeNames = Object.keys(card.securitySchemes ?? {});

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
          <p className="font-medium">Name</p>
          <p className="text-muted-foreground">{card.name}</p>
        </div>
        <div>
          <p className="font-medium">Description</p>
          <p className="text-muted-foreground">{card.description || "Not set"}</p>
        </div>
        <div>
          <p className="font-medium">Protocol</p>
          <p className="text-muted-foreground">{card.protocolVersion}</p>
        </div>
        <div>
          <p className="font-medium">Transport</p>
          <p className="text-muted-foreground">{card.preferredTransport}</p>
        </div>
        <div>
          <p className="font-medium">Authentication</p>
          <p className="text-muted-foreground">
            {securitySchemeNames.length ? securitySchemeNames.join(", ") : "Not advertised"}
          </p>
        </div>
        <div>
          <p className="font-medium">Session mode</p>
          <p className="text-muted-foreground">
            {getInvocationSessionModeDisplayName(props.sessionMode)}
          </p>
        </div>
      </div>

      <div className="mt-3">
        <p className="text-sm font-medium">Capabilities</p>
        <div className="mt-2 flex flex-wrap gap-1.5">
          {capabilityEntries.length ? (
            capabilityEntries.map(([key, value]) => (
              <Badge key={key} variant={value ? "default" : "secondary"}>
                {key}: {String(value)}
              </Badge>
            ))
          ) : (
            <p className="text-sm text-muted-foreground">None advertised</p>
          )}
        </div>
      </div>

      <div className="mt-3">
        <p className="text-sm font-medium">Skills</p>
        <div className="mt-2 space-y-2">
          {card.skills?.length ? (
            card.skills.map((skill, index) => (
              <div key={`${skill.id ?? "skill"}-${index}`} className="rounded-md border p-2">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-sm font-medium">
                    {skill.id ? (
                      <EntityIdentity value={skill.id}>{skill.name || skill.id}</EntityIdentity>
                    ) : (
                      skill.name || "Skill"
                    )}
                  </span>
                </div>
                {skill.description && (
                  <p className="mt-1 text-sm text-muted-foreground">{skill.description}</p>
                )}
                {!!skill.tags?.length && (
                  <div className="mt-2 flex flex-wrap gap-1">
                    {skill.tags.map((tag) => (
                      <Badge key={tag} variant="secondary">
                        {tag}
                      </Badge>
                    ))}
                  </div>
                )}
              </div>
            ))
          ) : (
            <p className="text-sm text-muted-foreground">None advertised</p>
          )}
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
