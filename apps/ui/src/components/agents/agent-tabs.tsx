import { BarChart3, Edit2, Eye, GitBranch, LayoutDashboard, LockKeyhole, Plug } from "lucide-react";
import type { SectionTabItem } from "@/components/layout";

const agentTabItems = {
  overview: {
    value: "overview",
    label: "Overview",
    icon: <LayoutDashboard className="size-4" />,
  },
  preview: { value: "preview", label: "Preview", icon: <Eye className="size-4" /> },
  credentials: {
    value: "credentials",
    label: "Credentials",
    icon: <LockKeyhole className="size-4" />,
  },
  // One tab, not three (EVE-1009). "Triggers" and "Integrate" both described a
  // slice of the same question — how is this agent reached, and when does it
  // run — and the second could only ever show generic snippets, because an
  // agent with two endpoints has no single URL. Both fold in here.
  integrations: {
    value: "integrations",
    label: "Integrations",
    icon: <Plug className="size-4" />,
  },
  versions: { value: "versions", label: "Versions", icon: <GitBranch className="size-4" /> },
  stats: { value: "stats", label: "Stats", icon: <BarChart3 className="size-4" /> },
  edit: { value: "edit", label: "Edit", icon: <Edit2 className="size-4" /> },
} satisfies Record<string, SectionTabItem>;

export function getAgentDetailTabItems(versionsEnabled: boolean): SectionTabItem[] {
  return [
    agentTabItems.overview,
    agentTabItems.preview,
    agentTabItems.credentials,
    agentTabItems.integrations,
    ...(versionsEnabled ? [agentTabItems.versions] : []),
    agentTabItems.stats,
  ];
}

export const agentEditTabItems: SectionTabItem[] = [agentTabItems.edit, agentTabItems.preview];
