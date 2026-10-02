"use client";

import { Pin } from "lucide-react";
import { useAgentVersions } from "@/hooks/use-agents";
import type { AgentVersion, AgentVersionPolicy, OpenApiAppChannel } from "@/lib/api/types";
import { Badge } from "@/components/ui/badge";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useFeatureFlag } from "@/providers/feature-flags-provider";

/// Which Agent version an exposure (endpoint or trigger) runs (EVE-1139).
export interface AgentVersionSelection {
  agent_version_policy: AgentVersionPolicy;
  agent_version_id: string | null;
}

export const DEFAULT_VERSION_SELECTION: AgentVersionSelection = {
  agent_version_policy: "default",
  agent_version_id: null,
};

type VersionSelectionFields = Pick<OpenApiAppChannel, "agent_version_policy" | "agent_version_id">;

/// Read an endpoint's or trigger's stored selection. Takes any record because
/// the hand-written `AppChannel` UI type predates these fields; the generated
/// schema is the source of their shape.
export function versionSelectionOf(value: object): AgentVersionSelection {
  const fields = value as VersionSelectionFields;
  return {
    agent_version_policy: fields.agent_version_policy ?? "default",
    agent_version_id: fields.agent_version_id ?? null,
  };
}

/// A pin needs a version; the other policies carry none.
export function isVersionSelectionValid(selection: AgentVersionSelection): boolean {
  return selection.agent_version_policy !== "pinned" || !!selection.agent_version_id;
}

/// Request fields for create/update. `default` and `latest` omit the id, which
/// the API reads as "clear the pin".
export function versionSelectionRequest(selection: AgentVersionSelection): {
  agent_version_policy: AgentVersionPolicy;
  agent_version_id?: string;
} {
  if (selection.agent_version_policy === "pinned" && selection.agent_version_id) {
    return { agent_version_policy: "pinned", agent_version_id: selection.agent_version_id };
  }
  return { agent_version_policy: selection.agent_version_policy };
}

export function sameVersionSelection(a: AgentVersionSelection, b: AgentVersionSelection): boolean {
  return (
    a.agent_version_policy === b.agent_version_policy &&
    (a.agent_version_policy !== "pinned" || a.agent_version_id === b.agent_version_id)
  );
}

function versionLabel(version: AgentVersion): string {
  return version.summary ? `${version.version} · ${version.summary}` : version.version;
}

export function describeVersionSelection(
  selection: AgentVersionSelection,
  versions: AgentVersion[],
): string {
  switch (selection.agent_version_policy) {
    case "latest":
      return "Latest saved version";
    case "pinned": {
      const version = versions.find(({ id }) => id === selection.agent_version_id);
      if (version) return `Pinned to ${version.version}`;
      return selection.agent_version_id ? "Pinned version" : "Pinned (no version)";
    }
    default:
      return "Agent default version";
  }
}

/// Whether the version control should show. The feature flag gates choosing a
/// policy, but a pin that already exists (for example one carried over from the
/// App era) stays visible so an org can always see and undo it.
export function useShowVersionSelection(selection: AgentVersionSelection): boolean {
  const enabled = useFeatureFlag("agent_versions");
  return enabled || selection.agent_version_policy !== "default";
}

export function AgentVersionPolicyField({
  agentId,
  value,
  onChange,
  disabled,
  idPrefix = "agent-version",
}: {
  agentId: string;
  value: AgentVersionSelection;
  onChange: (next: AgentVersionSelection) => void;
  disabled?: boolean;
  idPrefix?: string;
}) {
  const featureEnabled = useFeatureFlag("agent_versions");
  const { data: versions = [], isLoading } = useAgentVersions(agentId);
  // Automatic draft snapshots are history, not deployment targets; the API
  // rejects pinning them.
  const saved = versions.filter((version) => version.is_published);
  const pinnedMissing =
    value.agent_version_policy === "pinned" &&
    !!value.agent_version_id &&
    !saved.some(({ id }) => id === value.agent_version_id);
  // Without the feature only unpinning is allowed, so the existing value can be
  // cleared but no new one chosen.
  const policyLocked = disabled || (!featureEnabled && value.agent_version_policy === "default");

  return (
    <div className="space-y-3">
      <div className="space-y-2">
        <Label htmlFor={`${idPrefix}-policy`}>Runs</Label>
        <Select
          value={value.agent_version_policy}
          disabled={policyLocked}
          onValueChange={(policy) =>
            onChange({
              agent_version_policy: policy as AgentVersionPolicy,
              agent_version_id:
                policy === "pinned" ? (value.agent_version_id ?? saved[0]?.id ?? null) : null,
            })
          }
        >
          <SelectTrigger id={`${idPrefix}-policy`} className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="default">Agent default version</SelectItem>
            <SelectItem value="latest" disabled={!featureEnabled}>
              Latest saved version
            </SelectItem>
            <SelectItem value="pinned" disabled={!featureEnabled}>
              A pinned version
            </SelectItem>
          </SelectContent>
        </Select>
      </div>
      {value.agent_version_policy === "pinned" && (
        <div className="space-y-2">
          <Label htmlFor={`${idPrefix}-version`}>Pinned version</Label>
          <Select
            value={value.agent_version_id ?? undefined}
            disabled={disabled || !featureEnabled || isLoading}
            onValueChange={(agent_version_id) =>
              onChange({ agent_version_policy: "pinned", agent_version_id })
            }
          >
            <SelectTrigger id={`${idPrefix}-version`} className="w-full">
              <SelectValue placeholder={isLoading ? "Loading versions…" : "Choose a version"} />
            </SelectTrigger>
            <SelectContent>
              {pinnedMissing && value.agent_version_id && (
                <SelectItem value={value.agent_version_id}>Unavailable version</SelectItem>
              )}
              {saved.map((version) => (
                <SelectItem key={version.id} value={version.id}>
                  {versionLabel(version)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {!isLoading && saved.length === 0 && (
            <p className="text-xs text-muted-foreground">
              Save a version from Version history before pinning one.
            </p>
          )}
        </div>
      )}
      <p className="text-xs text-muted-foreground">
        {value.agent_version_policy === "pinned"
          ? "Edits to the agent do not reach this exposure until you change the pin."
          : value.agent_version_policy === "latest"
            ? "Runs the newest saved version, whatever the default is."
            : "Follows the version marked Default in Version history."}
      </p>
    </div>
  );
}

/// Compact read-only summary for lists: renders nothing for the default policy.
export function AgentVersionSelectionBadge({
  agentId,
  selection,
}: {
  agentId: string;
  selection: AgentVersionSelection;
}) {
  if (selection.agent_version_policy === "default") return null;
  return <NonDefaultVersionBadge agentId={agentId} selection={selection} />;
}

function NonDefaultVersionBadge({
  agentId,
  selection,
}: {
  agentId: string;
  selection: AgentVersionSelection;
}) {
  const { data: versions = [] } = useAgentVersions(agentId);
  return (
    <Badge variant="outline" className="gap-1">
      <Pin className="size-3" />
      {describeVersionSelection(selection, versions)}
    </Badge>
  );
}
