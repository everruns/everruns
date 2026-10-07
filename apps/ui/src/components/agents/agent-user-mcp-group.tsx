"use client";

// "User servers of the person chatting" in the agent MCP sheet.
// Spec: knowledge/integrations/user-mcp-servers.md (D2, UI).
//
// Shown when the agent has the `user_mcp` capability. The switches edit that
// capability's settings. Which servers join a turn depends on who is chatting,
// so the only list this sheet can show is the viewer's own: each of their
// servers whose name an agent or capability server already uses is reported
// as skipped, because the agent's server wins a name clash.

import { useState } from "react";
import { AlertTriangle, UserRound } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { LinkButton } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { useUpdateAgent } from "@/hooks/use-agents";
import { useUserMcpServers } from "@/hooks/use-user-mcp-servers";
import type { Agent, AgentMcpAttachment } from "@/lib/api/types";

export const USER_MCP_CAPABILITY_ID = "user_mcp";

type Setting = "use" | "manage" | "allow_custom_urls";

const SETTINGS: { key: Setting; label: string; help: string; fallback: boolean }[] = [
  {
    key: "use",
    label: "Use their servers",
    help: "The enabled MCP servers of the person chatting join their turns, signed in as them.",
    fallback: true,
  },
  {
    key: "manage",
    label: "Let the agent manage them",
    help: "The agent can list, add, remove, enable, disable and connect them in chat. Adding and enabling ask the person first.",
    fallback: false,
  },
  {
    key: "allow_custom_urls",
    label: "Allow servers outside the catalog",
    help: "With manage on, the agent may add a server by URL, not only from the MCP catalog.",
    fallback: false,
  },
];

function settingValue(config: Record<string, unknown>, setting: (typeof SETTINGS)[number]) {
  const value = config[setting.key];
  return typeof value === "boolean" ? value : setting.fallback;
}

export function AgentUserMcpGroup({
  agent,
  attachments,
}: {
  agent: Agent;
  attachments: AgentMcpAttachment[];
}) {
  const capability = agent.capabilities.find((c) => c.ref === USER_MCP_CAPABILITY_ID);
  if (!capability) return null;
  return <UserMcpGroup agent={agent} attachments={attachments} config={capability.config ?? {}} />;
}

function UserMcpGroup({
  agent,
  attachments,
  config,
}: {
  agent: Agent;
  attachments: AgentMcpAttachment[];
  config: Record<string, unknown>;
}) {
  const updateAgent = useUpdateAgent();
  const { data: myServers = [] } = useUserMcpServers("me");
  const [error, setError] = useState<string | null>(null);

  const readOnly = agent.is_built_in === true;
  const manage = settingValue(config, SETTINGS[1]);
  const byName = new Map(attachments.map((attachment) => [attachment.name, attachment]));
  const clashes = myServers.filter((server) => byName.has(server.name));

  const save = async (key: Setting, value: boolean) => {
    setError(null);
    const capabilities = agent.capabilities.map((c) =>
      c.ref === USER_MCP_CAPABILITY_ID ? { ...c, config: { ...c.config, [key]: value } } : c,
    );
    try {
      await updateAgent.mutateAsync({ agentId: agent.id, request: { capabilities } });
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "The request failed. Try again.");
    }
  };

  return (
    <Card>
      <CardHeader className="space-y-1">
        <CardTitle className="flex flex-wrap items-center gap-2">
          <UserRound className="size-4" />
          User servers of the person chatting
          <Badge variant="outline">Acts as the person</Badge>
        </CardTitle>
        <p className="text-sm text-muted-foreground">
          Each person&apos;s own servers from Settings &gt; My MCP servers. Not used by unattended
          runs or conversations with several people.
        </p>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="space-y-3">
          {SETTINGS.map((setting) => {
            const id = `user-mcp-${setting.key}`;
            const disabled =
              readOnly || updateAgent.isPending || (setting.key === "allow_custom_urls" && !manage);
            return (
              <div key={setting.key} className="flex items-start justify-between gap-4">
                <div className="space-y-0.5">
                  <Label htmlFor={id}>{setting.label}</Label>
                  <p className="text-xs text-muted-foreground">{setting.help}</p>
                </div>
                <Switch
                  id={id}
                  aria-label={setting.label}
                  checked={settingValue(config, setting)}
                  disabled={disabled}
                  onCheckedChange={(value) => save(setting.key, value)}
                />
              </div>
            );
          })}
          {readOnly && (
            <p className="text-xs text-muted-foreground">
              Built-in agent: these settings are managed by the platform.
            </p>
          )}
          {error && <p className="text-sm text-destructive">{error}</p>}
        </div>

        {clashes.length > 0 && (
          <ul className="space-y-1" aria-label="Skipped user servers">
            {clashes.map((server) => {
              const winner = byName.get(server.name);
              const kind = winner?.source === "capability" ? "capability" : "agent";
              return (
                <li
                  key={server.id}
                  className="flex items-start gap-2 text-xs text-muted-foreground"
                >
                  <AlertTriangle className="mt-0.5 size-3 shrink-0 text-destructive" />
                  <span>
                    Your server <span className="font-mono text-foreground">{server.name}</span> is
                    skipped here: name clash with {kind} server{" "}
                    <span className="font-mono text-foreground">{server.name}</span>, which is used
                    instead.
                  </span>
                </li>
              );
            })}
          </ul>
        )}

        <LinkButton href="/settings/agent-experience" variant="outline" size="sm">
          My MCP servers
        </LinkButton>
      </CardContent>
    </Card>
  );
}
