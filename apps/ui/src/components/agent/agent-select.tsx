"use client";

import { useMemo } from "react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useAgents } from "@/hooks";
import type { Agent } from "@/lib/api/types";

interface AgentSelectProps {
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
  className,
  disabled,
}: AgentSelectProps) {
  const { data: agents = [] } = useAgents();

  const agentMap = useMemo(() => {
    return new Map<string, Agent>(agents.map((a) => [a.id, a]));
  }, [agents]);

  // For the select, use "all" as the value for the all option
  const selectValue = includeAll && !value ? "all" : value;

  const handleChange = (newValue: string) => {
    onValueChange(newValue === "all" ? "" : newValue);
  };

  const displayValue = value ? agentMap.get(value)?.name : includeAll ? allLabel : undefined;

  return (
    <Select value={selectValue} onValueChange={handleChange} disabled={disabled}>
      <SelectTrigger className={className}>
        <SelectValue placeholder={placeholder}>{displayValue}</SelectValue>
      </SelectTrigger>
      <SelectContent>
        {includeAll && <SelectItem value="all">{allLabel}</SelectItem>}
        {agents.map((agent) => (
          <SelectItem key={agent.id} value={agent.id}>
            {agent.name}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}
