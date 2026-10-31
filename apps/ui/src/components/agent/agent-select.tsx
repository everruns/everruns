"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { useLocale as useDisplayLocale } from "@/providers/locale-provider";
import { useMemo } from "react";
import { Combobox } from "@/components/ui/combobox";
import { useAgents } from "@/hooks";
import type { Agent } from "@/lib/api/types";
import { getDisplayName } from "@/lib/entity-lifecycle";

interface AgentSelectProps {
  /** Accessible name for the trigger. */
  ariaLabel?: string;
  /** Selected agent ID */
  value: string;
  /** Callback when selection changes */
  onValueChange: (agentId: string) => void;
  /** Placeholder text when nothing selected */
  placeholder?: string;
  /** Include "All agents" option with empty string value */
  includeAll?: boolean;
  /** Label for "All agents" option */
  allLabel?: string;
  /** Include an explicit empty option */
  includeNoneOption?: boolean;
  /** Label for the empty option */
  noneLabel?: string;
  /** Additional className for trigger */
  className?: string;
  /** Disable the select */
  disabled?: boolean;
}

/**
 * Agent select dropdown that displays agent names but uses IDs as values.
 * Handles the ID-to-name mapping automatically.
 */
export function AgentSelect({
  value,
  onValueChange,
  placeholder = "Select agent",
  includeAll = false,
  allLabel = "All agents",
  includeNoneOption = false,
  noneLabel = "None",
  className,
  disabled,
  ariaLabel = "Agent",
}: AgentSelectProps) {
  const { displayLocale } = useDisplayLocale();
  const ui = useUiTranslations();
  const noneValue = "__none__";
  const { data: agents = [], isLoading, error } = useAgents();

  const agentMap = useMemo(() => {
    return new Map<string, Agent>(agents.map((a) => [a.id, a]));
  }, [agents]);

  // For the select, use "all" as the value for the all option
  const selectValue = value ? value : includeNoneOption ? noneValue : includeAll ? "all" : value;

  const handleChange = (newValue: string) => {
    onValueChange(newValue === "all" || newValue === noneValue ? "" : newValue);
  };

  const selectedAgent = value ? agentMap.get(value) : undefined;
  const displayValue = selectedAgent
    ? getDisplayName(selectedAgent, displayLocale)
    : includeNoneOption
      ? ui(noneLabel)
      : includeAll
        ? ui(allLabel)
        : undefined;

  return (
    <Combobox
      value={selectValue}
      onValueChange={handleChange}
      valueLabel={displayValue}
      ariaLabel={ui(ariaLabel)}
      placeholder={ui(placeholder)}
      searchPlaceholder={ui("Search agents")}
      disabled={disabled}
      loading={isLoading}
      error={error ? ui("Could not load agents. Try again.") : undefined}
      className={className}
      clearable={false}
      options={[
        ...(includeNoneOption ? [{ value: noneValue, label: ui(noneLabel) }] : []),
        ...(includeAll ? [{ value: "all", label: ui(allLabel) }] : []),
        ...agents.map((agent) => ({
          value: agent.id,
          label: getDisplayName(agent, displayLocale),
        })),
      ]}
    />
  );
}
