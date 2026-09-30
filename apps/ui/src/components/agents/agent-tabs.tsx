import { BarChart3, Boxes, Eye, MessageSquare, Plug } from "lucide-react";
import type { SectionTabItem } from "@/components/layout";

// One tab row for the agent page. MCP and Credentials are configuration, so
// they moved into the Agent tab's config column; Sessions got its own tab
// instead of a card on the overview; Versions lives in the header overflow.
// "Triggers" and "Integrate" folded into Integrations in EVE-1009: both
// described how an agent is reached and when it runs.
export type AgentTab = "agent" | "preview" | "integrations" | "stats" | "sessions";

export function getAgentTabItems(sessionCount?: number): SectionTabItem[] {
  return [
    { value: "agent", label: "Agent", icon: <Boxes className="size-4" /> },
    { value: "preview", label: "Preview", icon: <Eye className="size-4" /> },
    { value: "integrations", label: "Integrations", icon: <Plug className="size-4" /> },
    { value: "stats", label: "Stats", icon: <BarChart3 className="size-4" /> },
    {
      value: "sessions",
      label:
        sessionCount === undefined ? (
          "Sessions"
        ) : (
          <>
            Sessions
            <span className="bg-muted px-1 text-[11px] font-medium">{sessionCount}</span>
          </>
        ),
      icon: <MessageSquare className="size-4" />,
    },
  ];
}

const TABS = new Set<string>(["agent", "preview", "integrations", "stats", "sessions"]);

/**
 * Resolves a `?tab=` deep link. Tabs that became config-column sheets (MCP,
 * Credentials, Versions) still resolve, so existing return URLs keep working:
 * they land on the Agent tab with that sheet open.
 */
export function resolveAgentTab(param: string | null): {
  tab: AgentTab;
  section: "mcp" | "credentials" | "versions" | null;
} {
  if (param === "mcp" || param === "credentials" || param === "versions") {
    return { tab: "agent", section: param };
  }
  if (param && TABS.has(param)) return { tab: param as AgentTab, section: null };
  return { tab: "agent", section: null };
}
