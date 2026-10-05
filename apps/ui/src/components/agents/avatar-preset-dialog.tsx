"use client";

import { useState } from "react";
import { Check, Loader2, Search, X } from "lucide-react";
import { useAvatarPresets } from "@/hooks";
import { avatarPresetUrl } from "@/lib/api/agents";
import type { Agent } from "@/lib/api/types";
import { searchAvatarPresets } from "@/lib/avatar-presets";
import { AgentAvatar } from "./agent-avatar";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { cn } from "@/lib/utils";

export function AvatarPresetDialog({
  agent,
  open,
  disabled,
  saving,
  error,
  onClose,
  onSave,
}: {
  agent: Agent;
  open: boolean;
  disabled: boolean;
  saving: boolean;
  error: string | null;
  onClose: () => void;
  onSave: (id: string) => Promise<void>;
}) {
  const { catalog, current } = useAvatarPresets(agent.id, open);
  const [query, setQuery] = useState("");
  const [family, setFamily] = useState("");
  const [picked, setPicked] = useState<string | null>(null);
  const selected = picked ?? current.data?.preset_id;
  const presets = catalog.data ?? [];
  const matches = searchAvatarPresets(presets, query, family);
  const selectedPreset = presets.find((p) => p.id === selected);
  const clear = () => {
    setQuery("");
    setFamily("");
  };
  const close = () => {
    if (!saving) {
      setPicked(null);
      onClose();
    }
  };

  return (
    <Dialog open={open} onOpenChange={(value) => !value && close()}>
      <DialogContent className="flex max-h-[90dvh] flex-col sm:max-w-3xl" showCloseButton={!saving}>
        <DialogHeader>
          <DialogTitle>Choose an avatar</DialogTitle>
          <DialogDescription>
            Find a face, familiar or shape. These names and roles describe the artwork only.
          </DialogDescription>
        </DialogHeader>
        <div className="flex items-center gap-3">
          {agent.avatar && (
            <div
              className="flex items-center gap-2"
              aria-label="Current avatar in square and circle"
            >
              <AgentAvatar avatar={agent.avatar} size={40} />
              <AgentAvatar avatar={agent.avatar} size={40} shape="circle" />
            </div>
          )}
          <p className="text-xs text-muted-foreground">
            {agent.avatar ? "Current avatar" : "No avatar selected"}
          </p>
        </div>
        <div className="flex flex-col gap-2 sm:flex-row">
          <div className="relative flex-1">
            <Search className="absolute top-2 left-2 size-4 text-muted-foreground" aria-hidden />
            <Input
              aria-label="Search avatars"
              placeholder="Search names, roles, animals, colors…"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              className="pr-8 pl-8"
              disabled={saving}
            />
            {query && (
              <button
                type="button"
                aria-label="Clear search"
                className="absolute top-1.5 right-2 p-0.5"
                onClick={() => setQuery("")}
                disabled={saving}
              >
                <X className="size-4" />
              </button>
            )}
          </div>
          <select
            aria-label="Avatar family"
            value={family}
            onChange={(event) => setFamily(event.target.value)}
            disabled={saving}
            className="h-8 border border-input bg-background px-2 text-sm"
          >
            <option value="">All families</option>
            {[...new Set(presets.map((p) => p.family))].map((name) => (
              <option key={name} value={name}>
                {name}
              </option>
            ))}
          </select>
        </div>
        <p className="text-xs text-muted-foreground" role="status">
          {catalog.isLoading
            ? "Loading avatars…"
            : `${matches.length} avatar${matches.length === 1 ? "" : "s"}`}
        </p>
        {catalog.error ? (
          <div role="alert" className="text-sm text-destructive">
            Could not load avatars.{" "}
            <Button type="button" variant="outline" size="sm" onClick={() => catalog.refetch()}>
              Retry
            </Button>
          </div>
        ) : (
          <div className="min-h-0 flex-1 overflow-y-auto p-1">
            {!catalog.isLoading && matches.length === 0 ? (
              <div className="py-10 text-center">
                <p className="text-sm">No avatars match your search.</p>
                <Button type="button" variant="ghost" onClick={clear} disabled={saving}>
                  Clear filters
                </Button>
              </div>
            ) : (
              <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 md:grid-cols-5">
                {matches.map((preset) => (
                  <button
                    key={preset.id}
                    type="button"
                    disabled={disabled}
                    aria-pressed={selected === preset.id}
                    aria-label={`${preset.name}, ${preset.family}, ${preset.role}`}
                    onClick={() => setPicked(preset.id)}
                    className={cn(
                      "group relative flex flex-col items-start gap-1.5 border bg-card p-2 text-left transition-colors hover:bg-muted focus-visible:outline-2 focus-visible:outline-ring disabled:opacity-60",
                      selected === preset.id && "border-primary ring-1 ring-primary",
                    )}
                  >
                    {/* Canonical square preview; circles below use the same server renderer. */}
                    {/* eslint-disable-next-line @next/next/no-img-element */}
                    <img
                      src={avatarPresetUrl(preset)}
                      alt=""
                      width={128}
                      height={128}
                      className="aspect-square w-full object-cover"
                      loading="lazy"
                    />
                    <span className="text-sm font-medium">{preset.name}</span>
                    <span className="text-[11px] text-muted-foreground">
                      {preset.family} · {preset.role}
                    </span>
                    <span className="text-xs text-muted-foreground">{preset.description}</span>
                    {selected === preset.id && (
                      <Check
                        className="absolute top-3 right-3 size-5 border bg-background p-0.5"
                        aria-hidden
                      />
                    )}
                  </button>
                ))}
              </div>
            )}
          </div>
        )}
        {selectedPreset && (
          <div className="flex items-center gap-3 border-t pt-3">
            {/* eslint-disable-next-line @next/next/no-img-element */}
            <img
              src={avatarPresetUrl(selectedPreset, "circle")}
              alt={`${selectedPreset.name} circular preview`}
              width={48}
              height={48}
            />
            <p className="text-sm">
              {selectedPreset.name}
              <span className="block text-xs text-muted-foreground">
                Square and circle use the same artwork.
              </span>
            </p>
          </div>
        )}
        {(error ?? current.error?.message) && (
          <p role="alert" className="text-sm text-destructive">
            {error ?? current.error?.message}
          </p>
        )}
        <DialogFooter>
          <Button type="button" variant="outline" onClick={close} disabled={saving}>
            Cancel
          </Button>
          <Button
            type="button"
            disabled={disabled || !selectedPreset}
            onClick={async () => {
              if (disabled || !selected) return;
              try {
                await onSave(selected);
                setPicked(null);
              } catch {
                /* Mutation error stays visible for retry. */
              }
            }}
          >
            {saving && <Loader2 className="animate-spin" />}
            {saving ? "Saving avatar…" : "Use avatar"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
