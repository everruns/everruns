"use client";

import { useMemo } from "react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useVirtualUser, useVirtualUsers } from "@/hooks/use-virtual-users";
import { Button } from "@/components/ui/button";
import type { VirtualUser } from "@/lib/api/types";

interface VirtualUserSelectProps {
  value: string;
  onValueChange: (identityId: string) => void;
  placeholder?: string;
  includeNone?: boolean;
  noneLabel?: string;
  disabled?: boolean;
  /** Additional className for the trigger. */
  className?: string;
  usage?: "end_user" | "service";
}

export function VirtualUserSelect({
  value,
  onValueChange,
  placeholder = "Select virtual user",
  includeNone = true,
  noneLabel = "No identity",
  disabled,
  className,
  usage,
}: VirtualUserSelectProps) {
  const {
    data: identities = [],
    hasNextPage,
    fetchNextPage,
    isFetchingNextPage,
  } = useVirtualUsers({ includeArchived: !!value, usage });
  const { data: selectedUser } = useVirtualUser(value || undefined);
  const identityMap = useMemo(
    () => new Map<string, VirtualUser>(identities.map((identity) => [identity.id, identity])),
    [identities],
  );
  // When a value is set that's not in the list (e.g. deleted identity), show a fallback
  const selectedIdentity = value ? (identityMap.get(value) ?? selectedUser) : undefined;
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
      <SelectTrigger className={className} aria-label={placeholder}>
        <SelectValue placeholder={placeholder}>{displayLabel}</SelectValue>
      </SelectTrigger>
      <SelectContent>
        {includeNone && <SelectItem value="none">{noneLabel}</SelectItem>}
        {selectedIdentity && !identityMap.has(value) && (
          <SelectItem value={value} disabled={selectedIdentity.status !== "active"}>
            {selectedIdentity.name}
          </SelectItem>
        )}
        {/* Show a disabled entry for the current value if it's not in the list */}
        {valueMissing && (
          <SelectItem value={value} disabled>
            Unknown identity ({value.slice(0, 12)}...)
          </SelectItem>
        )}
        {identities
          .filter((identity) => !usage || identity.usage === usage)
          .map((identity) => (
            <SelectItem
              key={identity.id}
              value={identity.id}
              disabled={identity.status !== "active"}
            >
              {identity.name}
              {identity.status !== "active" ? " (archived)" : ""}
            </SelectItem>
          ))}
        {hasNextPage && (
          <Button
            variant="ghost"
            className="w-full"
            disabled={isFetchingNextPage}
            onClick={() => fetchNextPage()}
          >
            Load more virtual users
          </Button>
        )}
      </SelectContent>
    </Select>
  );
}
