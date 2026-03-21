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
  // When a value is set that's not in the list (e.g. deleted identity), show a fallback
  const selectedIdentity = value ? identityMap.get(value) : undefined;
  const valueMissing = !!value && !selectedIdentity;
  const displayLabel = selectedIdentity
    ? selectedIdentity.name
    : valueMissing
      ? `Unknown identity (${value.slice(0, 12)}...)`
      : includeNone
        ? noneLabel
        : undefined;
  const selectValue = includeNone && !value ? "none" : value;
  return (
    <Select
      value={selectValue}
      onValueChange={(next) => onValueChange(next === "none" ? "" : next)}
      disabled={disabled}
    >
      <SelectTrigger>
        <SelectValue placeholder={placeholder}>{displayLabel}</SelectValue>
      </SelectTrigger>
      <SelectContent>
        {includeNone && <SelectItem value="none">{noneLabel}</SelectItem>}
        {/* Show a disabled entry for the current value if it's not in the list */}
        {valueMissing && (
          <SelectItem value={value} disabled>
            Unknown identity ({value.slice(0, 12)}...)
          </SelectItem>
        )}
        {identities.map((identity) => (
          <SelectItem key={identity.id} value={identity.id} disabled={identity.status !== "active"}>
            {identity.name}
            {identity.status !== "active" ? " (archived)" : ""}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}
