"use client";
import { useUiTranslations } from "@/providers/locale-provider";
import { AgentSelect } from "./agent-select";
interface AgentFilterMenuProps {
  value: string;
  onValueChange: (agentId: string) => void;
  className?: string;
}
export function AgentFilterMenu(props: AgentFilterMenuProps) {
  const ui = useUiTranslations();
  return <AgentSelect {...props} includeAll ariaLabel={ui("Filter by agent")} />;
}
