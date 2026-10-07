import { BarChart3, Eye, Shield, Terminal } from "lucide-react";
import type { SectionTabItem } from "@/components/layout";

// One tab row for the harness page, matching the agent page: the definition is
// the first tab, and everything else is a sibling. Archive and delete live in
// the header overflow, not a tab or a danger-zone card.
export type HarnessTab = "harness" | "preview" | "integrate" | "stats";

export function getHarnessTabItems(): SectionTabItem[] {
  return [
    { value: "harness", label: "Harness", icon: <Shield className="size-4" /> },
    { value: "preview", label: "Preview", icon: <Eye className="size-4" /> },
    { value: "integrate", label: "Integrate", icon: <Terminal className="size-4" /> },
    { value: "stats", label: "Stats", icon: <BarChart3 className="size-4" /> },
  ];
}

const TABS = new Set<string>(["harness", "preview", "integrate", "stats"]);

/**
 * Resolves a `?tab=` deep link. The old Overview tab is the Harness workspace
 * now, so existing links land on it.
 */
export function resolveHarnessTab(param: string | null): HarnessTab {
  if (param === "overview") return "harness";
  if (param && TABS.has(param)) return param as HarnessTab;
  return "harness";
}
