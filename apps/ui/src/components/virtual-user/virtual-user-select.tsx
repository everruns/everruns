"use client";

import { useMemo, useState } from "react";
import { Combobox, type ComboboxOption } from "@/components/ui/combobox";
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
  const [search, setSearch] = useState("");
  const {
    isLoading,
    error,
    data: identities = [],
    hasNextPage,
    fetchNextPage,
    isFetchingNextPage,
  } = useVirtualUsers({ includeArchived: !!value, usage, search: search.trim() || undefined });
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
  const options: ComboboxOption[] = identities
    .filter((identity) => !usage || identity.usage === usage)
    .map((identity) => ({
      value: identity.id,
      label: identity.name + (identity.status !== "active" ? " (archived)" : ""),
      disabled: identity.status !== "active",
    }));
  if (!search.trim()) {
    if (includeNone) options.unshift({ value: "none", label: noneLabel });
    if (selectedIdentity && !identityMap.has(value)) {
      options.push({
        value,
        label: selectedIdentity.name,
        disabled:
          selectedIdentity.status !== "active" || (!!usage && selectedIdentity.usage !== usage),
      });
    } else if (valueMissing) {
      options.push({ value, label: displayLabel!, disabled: true });
    }
  }
  return (
    <Combobox
      value={includeNone && !value ? "none" : value}
      valueLabel={displayLabel}
      onValueChange={(next) => onValueChange(next === "none" ? "" : next)}
      options={options}
      placeholder={placeholder}
      searchPlaceholder="Search virtual users"
      disabled={disabled}
      clearable={false}
      className={className}
      onSearchChange={setSearch}
      loading={isLoading}
      error={error ? "Could not load virtual users. Try searching again." : undefined}
      footer={
        hasNextPage ? (
          <Button
            variant="ghost"
            className="w-full"
            disabled={isFetchingNextPage}
            onClick={() => fetchNextPage()}
          >
            {isFetchingNextPage ? "Loading…" : "Load more virtual users"}
          </Button>
        ) : undefined
      }
    />
  );
}
