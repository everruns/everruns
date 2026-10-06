import { BarChart3, Boxes, Eye, MessageSquare, Plug } from "lucide-react";
import type { SectionTabItem } from "@/components/layout";

// One tab row for the agent page. MCP and Credentials are configuration, so
// they moved into the Agent tab's config column; Sessions got its own tab
// instead of a card on the overview; History lives in the header overflow.
// "Triggers" and "Integrate" folded into Integrations in EVE-1009: both
// described how an agent is reached and when it runs.
export type AgentTab = "agent" | "preview" | "integrations" | "stats" | "sessions";

export function getAgentTabItems(
  sessionCount?: number,
  integrationCount?: number,
): SectionTabItem[] {
  return [
    { value: "agent", label: "Agent", icon: <Boxes className="size-4" /> },
    { value: "preview", label: "Preview", icon: <Eye className="size-4" /> },
    {
      value: "integrations",
      label:
        integrationCount === undefined ? (
          "Integrations"
        ) : (
          <>
            Integrations
            <span className="bg-muted px-1 text-[11px] font-medium">{integrationCount}</span>
          </>
        ),
      icon: <Plug className="size-4" />,
    },
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

export function isAgentTab(value: string): value is AgentTab {
  return TABS.has(value);
}

/**
 * Address for an agent page with `tab` selected. The Agent tab omits `tab` so
 * `/agents/{id}` stays the default; every other tab is in the query so a
 * refresh reopens it. Other params (for example `mode=edit`) are kept.
 */
export function agentTabHref(
  agentId: string,
  tab: AgentTab,
  search: { toString(): string },
): string {
  const params = new URLSearchParams(search.toString());
  if (tab === "agent") params.delete("tab");
  else params.set("tab", tab);
  const query = params.toString();
  return query ? `/agents/${agentId}?${query}` : `/agents/${agentId}`;
}

/**
 * Resolves a `?tab=` deep link. Tabs that became config-column sheets (MCP,
 * Credentials) still resolve, so existing return URLs keep working: they land
 * on the Agent tab with that sheet open. The retired Versions tab opens the
 * History record sheet (`?sheet=history`) instead.
 */
export function resolveAgentTab(param: string | null): {
  tab: AgentTab;
  section: "mcp" | "credentials" | null;
  recordSheet: "history" | null;
} {
  if (param === "mcp" || param === "credentials") {
    return { tab: "agent", section: param, recordSheet: null };
  }
  if (param === "versions") return { tab: "agent", section: null, recordSheet: "history" };
  if (param && TABS.has(param)) return { tab: param as AgentTab, section: null, recordSheet: null };
  return { tab: "agent", section: null, recordSheet: null };
}
