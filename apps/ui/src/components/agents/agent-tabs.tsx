import {
  BarChart3,
  Clock3,
  Edit2,
  Eye,
  GitBranch,
  LayoutDashboard,
  LockKeyhole,
  Terminal,
} from "lucide-react";
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
  triggers: { value: "triggers", label: "Triggers", icon: <Clock3 className="size-4" /> },
  versions: { value: "versions", label: "Versions", icon: <GitBranch className="size-4" /> },
  stats: { value: "stats", label: "Stats", icon: <BarChart3 className="size-4" /> },
  integrate: { value: "integrate", label: "Integrate", icon: <Terminal className="size-4" /> },
  edit: { value: "edit", label: "Edit", icon: <Edit2 className="size-4" /> },
} satisfies Record<string, SectionTabItem>;

export function getAgentDetailTabItems(versionsEnabled: boolean): SectionTabItem[] {
  return [
    agentTabItems.overview,
    agentTabItems.preview,
    agentTabItems.credentials,
    agentTabItems.triggers,
    ...(versionsEnabled ? [agentTabItems.versions] : []),
    agentTabItems.stats,
    agentTabItems.integrate,
  ];
}

export const agentEditTabItems: SectionTabItem[] = [agentTabItems.edit, agentTabItems.preview];
