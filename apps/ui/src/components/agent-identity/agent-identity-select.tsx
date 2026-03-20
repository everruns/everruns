"use client";

import { useMemo } from "react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useAgentIdentities } from "@/hooks/use-agent-identities";
import type { AgentIdentity } from "@/lib/api/types";

interface AgentIdentitySelectProps {
  value: string;
  onValueChange: (identityId: string) => void;
  placeholder?: string;
  includeNone?: boolean;
  noneLabel?: string;
  disabled?: boolean;
}

export function AgentIdentitySelect({
  value,
  onValueChange,
  placeholder = "Select agent identity",
  includeNone = true,
  noneLabel = "No identity",
  disabled,
}: AgentIdentitySelectProps) {
  const { data: identities = [] } = useAgentIdentities({ includeArchived: !!value });
  const identityMap = useMemo(
    () => new Map<string, AgentIdentity>(identities.map((identity) => [identity.id, identity])),
    [identities],
  );
  const selectValue = includeNone && !value ? "none" : value;
  return (
    <Select
      value={selectValue}
      onValueChange={(next) => onValueChange(next === "none" ? "" : next)}
      disabled={disabled}
    >
      <SelectTrigger>
        <SelectValue placeholder={placeholder}>
          {value ? identityMap.get(value)?.name : includeNone ? noneLabel : undefined}
        </SelectValue>
      </SelectTrigger>
      <SelectContent>
        {includeNone && <SelectItem value="none">{noneLabel}</SelectItem>}
        {identities.map((identity) => (
          <SelectItem
            key={identity.id}
            value={identity.id}
            disabled={identity.status !== "active"}
          >
            {identity.name}
            {identity.status !== "active" ? " (archived)" : ""}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}
