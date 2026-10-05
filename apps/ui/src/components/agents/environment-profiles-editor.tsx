"use client";

import { useMemo, useState } from "react";
import Link from "next/link";
import { Box, Check, Cpu, Plus, Star, Trash2, TriangleAlert } from "lucide-react";
import { useEnvironments, useEnvironmentTargets } from "@/hooks";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import type {
  EnvironmentProfile,
  EnvironmentSet,
  EnvironmentTargetDescriptor,
} from "@/lib/api/types";

const MAX_PROFILES = 16;
const DEFAULT_IDLE_SECONDS = 180;

const CAPABILITIES: Array<{
  key: keyof EnvironmentTargetDescriptor["capabilities"];
  label: string;
}> = [
  { key: "native_processes", label: "Native processes" },
  { key: "packages", label: "Packages" },
  { key: "pty", label: "Terminal" },
  { key: "ports", label: "Ports" },
  { key: "portable_checkpoint", label: "Portable recovery" },
];

function targetKey(target: Pick<EnvironmentTargetDescriptor, "kind" | "provider">): string {
  return `${target.kind}:${target.provider ?? ""}`;
}

function targetLabel(target: Pick<EnvironmentTargetDescriptor, "kind" | "provider">): string {
  if (target.kind === "vfs" && target.provider === "bashkit") return "Bashkit";
  if (target.kind === "managed" && target.provider === "daytona") return "Daytona";
  return target.provider ? `${target.provider} (${target.kind})` : target.kind;
}

function profileTargetKey(profile: EnvironmentProfile): string {
  return `${profile.target.kind}:${profile.target.provider ?? ""}`;
}

function targetOptions(profile: EnvironmentProfile): Record<string, unknown> {
  const options = profile.target.options;
  return options && typeof options === "object" && !Array.isArray(options) ? { ...options } : {};
}

export function createEnvironmentProfile(target: EnvironmentTargetDescriptor): EnvironmentProfile {
  return {
    target: {
      kind: target.kind as EnvironmentProfile["target"]["kind"],
      ...(target.provider ? { provider: target.provider } : {}),
    },
    durability: target.durability as EnvironmentProfile["durability"],
    lifecycle: {
      idle_after_seconds: DEFAULT_IDLE_SECONDS,
      idle_action: "checkpoint_and_stop",
    },
    bootstrap: { commands: [] },
  };
}

export function nextEnvironmentName(
  target: EnvironmentTargetDescriptor,
  profiles: Record<string, EnvironmentProfile>,
): string {
  const base = target.provider || target.kind || "environment";
  if (!profiles[base]) return base;
  for (let suffix = 2; suffix <= MAX_PROFILES + 1; suffix += 1) {
    const candidate = `${base}-${suffix}`;
    if (!profiles[candidate]) return candidate;
  }
  return `environment-${Object.keys(profiles).length + 1}`;
}

function validateProfileName(name: string, current: string, profiles: Record<string, unknown>) {
  if (!name) return "Enter a name.";
  if (name.length > 64) return "Use at most 64 characters.";
  if (!/^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/.test(name) || name.includes("--")) {
    return "Use lowercase letters, numbers, and single hyphens.";
  }
  if (name !== current && profiles[name]) return "That name is already used.";
  return null;
}

function ProfileNameInput({
  name,
  profiles,
  disabled,
  onRename,
}: {
  name: string;
  profiles: Record<string, EnvironmentProfile>;
  disabled: boolean;
  onRename: (name: string) => void;
}) {
  const [draft, setDraft] = useState(name);
  const error = validateProfileName(draft, name, profiles);

  const commit = () => {
    if (!error && draft !== name) onRename(draft);
    else if (error) setDraft(name);
  };

  return (
    <div className="min-w-0 flex-1 space-y-1">
      <Input
        aria-label={`Environment profile name ${name}`}
        value={draft}
        onChange={(event) => setDraft(event.target.value.toLowerCase())}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            commit();
          }
        }}
        disabled={disabled}
        className="font-mono"
      />
      {error && draft !== name ? <p className="text-xs text-destructive">{error}</p> : null}
    </div>
  );
}

function ProfileEditor({
  name,
  profile,
  profiles,
  isDefault,
  targets,
  disabled,
  onChange,
  onRename,
  onDefault,
  onRemove,
}: {
  name: string;
  profile: EnvironmentProfile;
  profiles: Record<string, EnvironmentProfile>;
  isDefault: boolean;
  targets: EnvironmentTargetDescriptor[];
  disabled: boolean;
  onChange: (profile: EnvironmentProfile) => void;
  onRename: (name: string) => void;
  onDefault: () => void;
  onRemove: () => void;
}) {
  const descriptor = targets.find((target) => targetKey(target) === profileTargetKey(profile));
  const options = targetOptions(profile);
  const managed = profile.target.kind === "managed";
  const lifecycle = {
    idle_after_seconds: profile.lifecycle?.idle_after_seconds ?? DEFAULT_IDLE_SECONDS,
    idle_action: profile.lifecycle?.idle_action ?? "checkpoint_and_stop",
  };

  const setOption = (key: string, value: unknown) => {
    const next = targetOptions(profile);
    if (value === "" || value === undefined) delete next[key];
    else next[key] = value;
    onChange({ ...profile, target: { ...profile.target, options: next } });
  };

  return (
    <section className="space-y-5 border bg-background p-4" aria-label={`Environment ${name}`}>
      <div className="flex items-start gap-2">
        <ProfileNameInput name={name} profiles={profiles} disabled={disabled} onRename={onRename} />
        <Button
          type="button"
          variant={isDefault ? "secondary" : "outline"}
          size="sm"
          onClick={onDefault}
          disabled={disabled || isDefault}
          aria-label={isDefault ? `${name} is the default environment` : `Make ${name} default`}
        >
          <Star className="size-3.5" />
          {isDefault ? "Default" : "Make default"}
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          onClick={onRemove}
          disabled={disabled}
          aria-label={`Remove ${name}`}
        >
          <Trash2 className="size-4" />
        </Button>
      </div>

      <div className="space-y-2">
        <Label htmlFor={`environment-target-${name}`}>Runs in</Label>
        <Select
          value={profileTargetKey(profile)}
          onValueChange={(value) => {
            const target = targets.find((candidate) => targetKey(candidate) === value);
            if (!target) return;
            const replacement = createEnvironmentProfile(target);
            onChange({
              ...replacement,
              lifecycle: profile.lifecycle ?? replacement.lifecycle,
              bootstrap:
                target.kind === "managed"
                  ? (profile.bootstrap ?? replacement.bootstrap)
                  : { commands: [] },
            });
          }}
          disabled={disabled}
        >
          <SelectTrigger id={`environment-target-${name}`} className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {targets.map((target) => (
              <SelectItem
                key={targetKey(target)}
                value={targetKey(target)}
                disabled={!target.available}
              >
                {targetLabel(target)}
                {target.available ? "" : " — unavailable"}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {descriptor && !descriptor.available ? (
          <p className="flex items-start gap-1.5 text-xs text-destructive">
            <TriangleAlert className="mt-0.5 size-3.5 shrink-0" />
            {descriptor.reason || "This target is unavailable on this deployment."}
          </p>
        ) : null}
      </div>

      {descriptor ? (
        <div className="space-y-2">
          <div className="flex flex-wrap gap-1.5">
            {CAPABILITIES.map(({ key, label }) => (
              <Badge key={key} variant={descriptor.capabilities[key] ? "secondary" : "outline"}>
                {descriptor.capabilities[key] ? <Check className="size-3" /> : null}
                {label}
              </Badge>
            ))}
          </div>
          <p className="text-xs text-muted-foreground">
            Recovery: {descriptor.durability.replaceAll("_", " ")} · Containment:{" "}
            {descriptor.containment_levels.join(", ")}
          </p>
        </div>
      ) : null}

      {managed ? (
        <div className="space-y-4">
          <p className="text-xs text-muted-foreground">
            Daytona uses the connection of the person starting the chat. Configure it in{" "}
            <Link href="/settings/agent-experience" className="underline underline-offset-2">
              Settings → My agent experience
            </Link>
            .
          </p>
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor={`environment-size-${name}`}>Compute size</Label>
              <Select
                value={typeof options.size === "string" ? options.size : "default"}
                onValueChange={(value) => setOption("size", value === "default" ? "" : value)}
                disabled={disabled}
              >
                <SelectTrigger id={`environment-size-${name}`} className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="default">Provider default</SelectItem>
                  <SelectItem value="small">Small</SelectItem>
                  <SelectItem value="medium">Medium</SelectItem>
                  <SelectItem value="large">Large</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-2">
              <Label htmlFor={`environment-snapshot-${name}`}>Snapshot</Label>
              <Input
                id={`environment-snapshot-${name}`}
                value={typeof options.snapshot === "string" ? options.snapshot : ""}
                onChange={(event) => setOption("snapshot", event.target.value)}
                placeholder="Provider default"
                disabled={disabled}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor={`environment-workspace-${name}`}>Workspace path</Label>
              <Input
                id={`environment-workspace-${name}`}
                value={typeof options.workspace_path === "string" ? options.workspace_path : ""}
                onChange={(event) => setOption("workspace_path", event.target.value)}
                placeholder="/home/daytona/workspace"
                disabled={disabled}
                className="font-mono"
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor={`environment-auto-stop-${name}`}>Provider auto-stop (minutes)</Label>
              <Input
                id={`environment-auto-stop-${name}`}
                type="number"
                min={1}
                max={60}
                value={
                  typeof options.auto_stop_minutes === "number" ? options.auto_stop_minutes : ""
                }
                onChange={(event) =>
                  setOption(
                    "auto_stop_minutes",
                    event.target.value ? Number(event.target.value) : "",
                  )
                }
                placeholder="Provider default"
                disabled={disabled}
              />
            </div>
          </div>
        </div>
      ) : null}

      <div className="grid gap-4 sm:grid-cols-2">
        <div className="space-y-2">
          <Label htmlFor={`environment-idle-action-${name}`}>When idle</Label>
          <Select
            value={lifecycle.idle_action}
            onValueChange={(idle_action) =>
              onChange({
                ...profile,
                lifecycle: {
                  ...lifecycle,
                  idle_action: idle_action as typeof lifecycle.idle_action,
                },
              })
            }
            disabled={disabled}
          >
            <SelectTrigger id={`environment-idle-action-${name}`} className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="checkpoint_and_stop">Checkpoint and stop</SelectItem>
              <SelectItem value="stop">Stop</SelectItem>
              <SelectItem value="keep_running">Keep running</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-2">
          <Label htmlFor={`environment-idle-after-${name}`}>Idle after (seconds)</Label>
          <Input
            id={`environment-idle-after-${name}`}
            type="number"
            min={1}
            max={86_400}
            value={lifecycle.idle_after_seconds}
            onChange={(event) =>
              onChange({
                ...profile,
                lifecycle: {
                  ...lifecycle,
                  idle_after_seconds: Number(event.target.value),
                },
              })
            }
            disabled={disabled || lifecycle.idle_action === "keep_running"}
          />
        </div>
      </div>

      {managed ? (
        <div className="space-y-2">
          <Label htmlFor={`environment-bootstrap-${name}`}>Bootstrap commands</Label>
          <Textarea
            id={`environment-bootstrap-${name}`}
            value={(profile.bootstrap?.commands ?? []).join("\n")}
            onChange={(event) =>
              onChange({
                ...profile,
                bootstrap: {
                  commands: event.target.value
                    .split("\n")
                    .map((command) => command.trim())
                    .filter(Boolean),
                },
              })
            }
            placeholder={"git clone …\npnpm install"}
            rows={3}
            disabled={disabled}
            className="font-mono text-xs"
          />
          <p className="text-xs text-muted-foreground">
            Runs once when Everruns creates replacement compute. Do not put credentials here.
          </p>
        </div>
      ) : null}
    </section>
  );
}

export function EnvironmentProfilesEditor({
  value,
  onChange,
  disabled = false,
  definitionMode = false,
}: {
  value: EnvironmentSet | null;
  onChange: (value: EnvironmentSet | null) => void;
  disabled?: boolean;
  /** Edit one reusable Environment definition, not an Agent binding policy. */
  definitionMode?: boolean;
}) {
  const { data, isLoading, error } = useEnvironmentTargets();
  const { data: reusableEnvironments = [] } = useEnvironments();
  const targets = useMemo(() => data?.items ?? [], [data?.items]);
  const profiles = value?.profiles ?? {};
  const entries = Object.entries(profiles);

  const add = (target: EnvironmentTargetDescriptor) => {
    const name = nextEnvironmentName(target, profiles);
    onChange({
      policy: value?.policy ?? "fixed",
      default: value?.default || name,
      profiles: { ...profiles, [name]: createEnvironmentProfile(target) },
    });
  };

  const addReusable = (environment: (typeof reusableEnvironments)[number]) => {
    const base = environment.name;
    const name = profiles[base]
      ? nextEnvironmentName(
          environment.current_revision.profile.target as EnvironmentTargetDescriptor,
          profiles,
        )
      : base;
    onChange({
      policy: value?.policy ?? "fixed",
      default: value?.default || name,
      profiles: {
        ...profiles,
        [name]: {
          ...environment.current_revision.profile,
          source_revision_id: environment.current_revision.id,
        },
      },
    });
  };

  const update = (name: string, profile: EnvironmentProfile) => {
    onChange({
      policy: value?.policy ?? "fixed",
      default: value?.default || name,
      profiles: { ...profiles, [name]: profile },
    });
  };

  const rename = (from: string, to: string) => {
    const renamed: Record<string, EnvironmentProfile> = {};
    for (const [name, profile] of Object.entries(profiles)) {
      renamed[name === from ? to : name] = profile;
    }
    onChange({
      policy: value?.policy ?? "fixed",
      default: value?.default === from ? to : value?.default || to,
      profiles: renamed,
    });
  };

  const remove = (name: string) => {
    const remaining = Object.fromEntries(
      Object.entries(profiles).filter(([profileName]) => profileName !== name),
    );
    const names = Object.keys(remaining);
    if (names.length === 0) {
      onChange(null);
      return;
    }
    onChange({
      policy: value?.policy ?? "fixed",
      default: value?.default === name ? names[0] : value?.default || names[0],
      profiles: remaining,
    });
  };

  const availableTargets = targets.filter((target) => target.available);

  return (
    <div className="space-y-5">
      <div className="border bg-muted/40 p-4">
        <div className="flex items-start gap-3">
          <Box className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
          <div className="space-y-1">
            <p className="text-sm font-medium">Filesystem plus replaceable compute</p>
            <p className="text-xs text-muted-foreground">
              New Playground sessions pin one named profile. Checkpointed profiles restore the
              workspace into replacement compute if the physical sandbox disappears; running
              processes do not survive replacement.
            </p>
          </div>
        </div>
      </div>

      {!definitionMode ? (
        <div className="space-y-2">
          <Label htmlFor="sandbox-policy">Session choice</Label>
          <Select
            value={value?.policy ?? "fixed"}
            onValueChange={(policy) =>
              onChange(value ? { ...value, policy: policy as EnvironmentSet["policy"] } : value)
            }
            disabled={disabled || !value}
          >
            <SelectTrigger id="sandbox-policy">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="fixed" disabled={entries.length > 1}>
                Fixed — sessions cannot override
              </SelectItem>
              <SelectItem value="selectable">Selectable — declared environments only</SelectItem>
              <SelectItem value="configurable">
                Configurable — allow constrained one-offs
              </SelectItem>
            </SelectContent>
          </Select>
          <p className="text-xs text-muted-foreground">
            Fixed is the safe default. Playground only shows a picker for selectable or configurable
            Agents.
          </p>
        </div>
      ) : null}

      {entries.map(([name, profile]) => (
        <ProfileEditor
          key={name}
          name={name}
          profile={profile}
          profiles={profiles}
          isDefault={value?.default === name}
          targets={targets}
          disabled={disabled}
          onChange={(next) => update(name, { ...next, source_revision_id: undefined })}
          onRename={(next) => rename(name, next)}
          onDefault={() => onChange({ policy: value?.policy ?? "fixed", default: name, profiles })}
          onRemove={() => remove(name)}
        />
      ))}

      {error ? (
        <p className="flex items-start gap-1.5 text-sm text-destructive">
          <TriangleAlert className="mt-0.5 size-4 shrink-0" />
          Could not load the execution targets offered by this deployment.
        </p>
      ) : null}

      {!disabled && entries.length < MAX_PROFILES ? (
        <div className="space-y-2">
          {!definitionMode && reusableEnvironments.length > 0 ? (
            <>
              <p className="text-xs font-medium text-muted-foreground">
                Use a reusable environment
              </p>
              <div className="flex flex-wrap gap-2">
                {reusableEnvironments.map((environment) => (
                  <Button
                    key={environment.id}
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() => addReusable(environment)}
                    disabled={(value?.policy ?? "fixed") === "fixed" && entries.length > 0}
                  >
                    <Box className="size-4" />
                    <Plus className="size-3" />
                    {environment.display_name}
                  </Button>
                ))}
              </div>
              <Link href="/environments" className="text-xs text-muted-foreground underline">
                Manage reusable environments
              </Link>
            </>
          ) : null}
          <p className="text-xs font-medium text-muted-foreground">Add an environment</p>
          <div className="flex flex-wrap gap-2">
            {isLoading ? (
              <span className="text-sm text-muted-foreground">Loading targets…</span>
            ) : null}
            {availableTargets.map((target) => (
              <Button
                key={targetKey(target)}
                type="button"
                variant="outline"
                size="sm"
                onClick={() => add(target)}
                disabled={(value?.policy ?? "fixed") === "fixed" && entries.length > 0}
              >
                {target.kind === "vfs" ? <Box className="size-4" /> : <Cpu className="size-4" />}
                <Plus className="size-3" />
                {targetLabel(target)}
              </Button>
            ))}
          </div>
          {targets
            .filter((target) => !target.available)
            .map((target) => (
              <p key={targetKey(target)} className="text-xs text-muted-foreground">
                {targetLabel(target)} is unavailable: {target.reason || "not installed"}.
              </p>
            ))}
        </div>
      ) : null}
    </div>
  );
}
