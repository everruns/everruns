"use client";

import { useMemo, useState } from "react";
import Link from "next/link";
import { Box, Check, Cpu, Plus, Star, Trash2, TriangleAlert } from "lucide-react";
import { useSandboxTargets, useSandboxTemplates } from "@/hooks";
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
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import type { SandboxTemplateSpec, SandboxPolicy, SandboxTargetDescriptor } from "@/lib/api/types";

const MAX_TEMPLATES = 16;
const DEFAULT_IDLE_SECONDS = 180;

const CAPABILITIES: Array<{
  key: keyof SandboxTargetDescriptor["capabilities"];
  label: string;
}> = [
  { key: "native_processes", label: "Native processes" },
  { key: "packages", label: "Packages" },
  { key: "pty", label: "Terminal" },
  { key: "ports", label: "Ports" },
  { key: "portable_checkpoint", label: "Portable recovery" },
];

function targetKey(target: Pick<SandboxTargetDescriptor, "kind" | "provider">): string {
  return `${target.kind}:${target.provider ?? ""}`;
}

function targetLabel(target: Pick<SandboxTargetDescriptor, "kind" | "provider">): string {
  if (target.kind === "vfs" && target.provider === "bashkit") return "Bashkit";
  if (target.kind === "managed" && target.provider === "daytona") return "Daytona";
  if (target.kind === "managed" && target.provider === "modal") return "Modal";
  return target.provider ? `${target.provider} (${target.kind})` : target.kind;
}

function specTargetKey(spec: SandboxTemplateSpec): string {
  return `${spec.target.kind}:${spec.target.provider ?? ""}`;
}

function targetOptions(spec: SandboxTemplateSpec): Record<string, unknown> {
  const options = spec.target.options;
  return options && typeof options === "object" && !Array.isArray(options) ? { ...options } : {};
}

export function createSandboxTemplateSpec(target: SandboxTargetDescriptor): SandboxTemplateSpec {
  return {
    target: {
      kind: target.kind as SandboxTemplateSpec["target"]["kind"],
      ...(target.provider ? { provider: target.provider } : {}),
    },
    durability: target.durability as SandboxTemplateSpec["durability"],
    lifecycle: {
      idle_after_seconds: DEFAULT_IDLE_SECONDS,
      idle_action: "checkpoint_and_stop",
    },
    bootstrap: { commands: [] },
  };
}

export function nextSandboxBindingName(
  target: SandboxTargetDescriptor,
  templates: Record<string, SandboxTemplateSpec>,
): string {
  const base = target.provider || target.kind || "sandbox";
  if (!templates[base]) return base;
  for (let suffix = 2; suffix <= MAX_TEMPLATES + 1; suffix += 1) {
    const candidate = `${base}-${suffix}`;
    if (!templates[candidate]) return candidate;
  }
  return `sandbox-${Object.keys(templates).length + 1}`;
}

function validateBindingName(name: string, current: string, templates: Record<string, unknown>) {
  if (!name) return "Enter a name.";
  if (name.length > 64) return "Use at most 64 characters.";
  if (!/^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/.test(name) || name.includes("--")) {
    return "Use lowercase letters, numbers, and single hyphens.";
  }
  if (name !== current && templates[name]) return "That name is already used.";
  return null;
}

function TemplateBindingNameInput({
  name,
  templates,
  disabled,
  onRename,
}: {
  name: string;
  templates: Record<string, SandboxTemplateSpec>;
  disabled: boolean;
  onRename: (name: string) => void;
}) {
  const [draft, setDraft] = useState(name);
  const error = validateBindingName(draft, name, templates);

  const commit = () => {
    if (!error && draft !== name) onRename(draft);
    else if (error) setDraft(name);
  };

  return (
    <div className="min-w-0 flex-1 space-y-1">
      <Input
        aria-label={`Sandbox binding name ${name}`}
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

function optionString(options: Record<string, unknown>, key: string): string {
  const value = options[key];
  return typeof value === "string" || typeof value === "number" ? String(value) : "";
}

/** Modal fields; the server and provider validate the same ranges. */
type NetworkPolicy = NonNullable<NonNullable<SandboxTemplateSpec["containment"]>["network"]>;

function ModalOptionFields({
  name,
  options,
  network,
  disabled,
  setOption,
  onNetworkChange,
}: {
  name: string;
  options: Record<string, unknown>;
  network: NetworkPolicy;
  disabled: boolean;
  setOption: (key: string, value: unknown) => void;
  onNetworkChange: (network: NetworkPolicy) => void;
}) {
  const injected = Array.isArray(options.inject_connections) ? options.inject_connections : [];
  const injectGithub = injected.includes("github");
  // Modal cannot inject credentials when egress is blocked or limited to domains.
  const injectionAllowed = network.mode === "allow";
  const numeric = (key: string) => (event: React.ChangeEvent<HTMLInputElement>) =>
    setOption(key, event.target.value ? Number(event.target.value) : "");
  return (
    <div className="grid gap-4 sm:grid-cols-2">
      <div className="space-y-2">
        <Label htmlFor={`sandbox-runtime-${name}`}>Runtime</Label>
        <Select
          value={typeof options.runtime === "string" ? options.runtime : "vm"}
          onValueChange={(value) => setOption("runtime", value === "vm" ? "" : value)}
          disabled={disabled}
        >
          <SelectTrigger id={`sandbox-runtime-${name}`} className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="vm">VM (own kernel)</SelectItem>
            <SelectItem value="gvisor">gVisor container</SelectItem>
          </SelectContent>
        </Select>
      </div>
      <div className="space-y-2">
        <Label htmlFor={`sandbox-image-${name}`}>Image</Label>
        <Input
          id={`sandbox-image-${name}`}
          value={optionString(options, "image")}
          onChange={(event) => setOption("image", event.target.value)}
          placeholder="python:3.13-slim with git and curl"
          disabled={disabled}
          className="font-mono"
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor={`sandbox-cpu-${name}`}>CPU cores</Label>
        <Input
          id={`sandbox-cpu-${name}`}
          type="number"
          min={0.125}
          max={64}
          step={0.125}
          value={optionString(options, "cpu")}
          onChange={numeric("cpu")}
          placeholder="Provider default"
          disabled={disabled}
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor={`sandbox-memory-${name}`}>Memory (MiB)</Label>
        <Input
          id={`sandbox-memory-${name}`}
          type="number"
          min={128}
          max={262_144}
          value={optionString(options, "memory_mb")}
          onChange={numeric("memory_mb")}
          placeholder="Provider default"
          disabled={disabled}
        />
      </div>
      <div className="space-y-2 sm:col-span-2">
        <Label htmlFor={`sandbox-workspace-${name}`}>Workspace path</Label>
        <Input
          id={`sandbox-workspace-${name}`}
          value={optionString(options, "workspace_path")}
          onChange={(event) => setOption("workspace_path", event.target.value)}
          placeholder="/workspace"
          disabled={disabled}
          className="font-mono"
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor={`sandbox-network-${name}`}>Outbound network</Label>
        <Select
          value={network.mode}
          onValueChange={(mode) =>
            onNetworkChange(
              mode === "allowlist"
                ? { mode: "allowlist", allowed_hosts: [] }
                : { mode: mode as "allow" | "deny" },
            )
          }
          disabled={disabled}
        >
          <SelectTrigger id={`sandbox-network-${name}`} className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="allow">Open</SelectItem>
            <SelectItem value="allowlist">Only listed domains</SelectItem>
            <SelectItem value="deny">Blocked</SelectItem>
          </SelectContent>
        </Select>
      </div>
      <div className="flex items-start gap-3 pt-6">
        <Switch
          id={`sandbox-inject-github-${name}`}
          aria-label="Use my GitHub connection"
          checked={injectGithub && injectionAllowed}
          onCheckedChange={(checked) => setOption("inject_connections", checked ? ["github"] : "")}
          disabled={disabled || !injectionAllowed}
        />
        <p className="text-xs text-muted-foreground">
          Use my GitHub connection for api.github.com and git. Modal adds the token to requests, so
          the agent never sees it.
          {injectionAllowed ? "" : " Needs open network."}
        </p>
      </div>
      {network.mode === "allowlist" ? (
        <div className="space-y-2 sm:col-span-2">
          <Label htmlFor={`sandbox-domains-${name}`}>Allowed domains</Label>
          <Textarea
            id={`sandbox-domains-${name}`}
            value={network.allowed_hosts.join("\n")}
            onChange={(event) =>
              onNetworkChange({
                mode: "allowlist",
                allowed_hosts: event.target.value
                  .split("\n")
                  .map((host) => host.trim())
                  .filter(Boolean),
              })
            }
            placeholder={"pypi.org\n*.pythonhosted.org"}
            disabled={disabled}
            className="font-mono"
          />
        </div>
      ) : null}
    </div>
  );
}

function TemplateSpecEditor({
  name,
  spec,
  templates,
  isDefault,
  targets,
  disabled,
  onChange,
  onRename,
  onDefault,
  onRemove,
}: {
  name: string;
  spec: SandboxTemplateSpec;
  templates: Record<string, SandboxTemplateSpec>;
  isDefault: boolean;
  targets: SandboxTargetDescriptor[];
  disabled: boolean;
  onChange: (spec: SandboxTemplateSpec) => void;
  onRename: (name: string) => void;
  onDefault: () => void;
  onRemove: () => void;
}) {
  const descriptor = targets.find((target) => targetKey(target) === specTargetKey(spec));
  const options = targetOptions(spec);
  const managed = spec.target.kind === "managed";
  const modal = managed && spec.target.provider === "modal";
  const lifecycle = {
    idle_after_seconds: spec.lifecycle?.idle_after_seconds ?? DEFAULT_IDLE_SECONDS,
    idle_action: spec.lifecycle?.idle_action ?? "checkpoint_and_stop",
  };

  const setOption = (key: string, value: unknown) => {
    const next = targetOptions(spec);
    if (value === "" || value === undefined) delete next[key];
    else next[key] = value;
    onChange({ ...spec, target: { ...spec.target, options: next } });
  };

  return (
    <section className="space-y-5 border bg-background p-4" aria-label={`Sandbox ${name}`}>
      <div className="flex items-start gap-2">
        <TemplateBindingNameInput
          name={name}
          templates={templates}
          disabled={disabled}
          onRename={onRename}
        />
        <Button
          type="button"
          variant={isDefault ? "secondary" : "outline"}
          size="sm"
          onClick={onDefault}
          disabled={disabled || isDefault}
          aria-label={isDefault ? `${name} is the default sandbox` : `Make ${name} default`}
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
        <Label htmlFor={`sandbox-target-${name}`}>Runs in</Label>
        <Select
          value={specTargetKey(spec)}
          onValueChange={(value) => {
            const target = targets.find((candidate) => targetKey(candidate) === value);
            if (!target) return;
            const replacement = createSandboxTemplateSpec(target);
            onChange({
              ...replacement,
              lifecycle: spec.lifecycle ?? replacement.lifecycle,
              bootstrap:
                target.kind === "managed"
                  ? (spec.bootstrap ?? replacement.bootstrap)
                  : { commands: [] },
            });
          }}
          disabled={disabled}
        >
          <SelectTrigger id={`sandbox-target-${name}`} className="w-full">
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
            {modal ? "Modal" : "Daytona"} uses the connection of the person starting the chat.
            Configure it in{" "}
            <Link href="/settings/agent-experience" className="underline underline-offset-2">
              Settings → My agent experience
            </Link>
            .
          </p>
          {modal ? (
            <ModalOptionFields
              name={name}
              options={options}
              network={spec.containment?.network ?? { mode: "allow" }}
              disabled={disabled}
              setOption={setOption}
              onNetworkChange={(network) => {
                const next = {
                  ...spec,
                  containment: { level: "isolated" as const, ...spec.containment, network },
                };
                // Injection needs open egress; drop it rather than save a template Modal rejects.
                if (network.mode !== "allow") {
                  const nextOptions = targetOptions(spec);
                  delete nextOptions.inject_connections;
                  next.target = { ...spec.target, options: nextOptions };
                }
                onChange(next);
              }}
            />
          ) : (
            <div className="grid gap-4 sm:grid-cols-2">
              <div className="space-y-2">
                <Label htmlFor={`sandbox-size-${name}`}>Compute size</Label>
                <Select
                  value={typeof options.size === "string" ? options.size : "default"}
                  onValueChange={(value) => setOption("size", value === "default" ? "" : value)}
                  disabled={disabled}
                >
                  <SelectTrigger id={`sandbox-size-${name}`} className="w-full">
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
                <Label htmlFor={`sandbox-snapshot-${name}`}>Snapshot</Label>
                <Input
                  id={`sandbox-snapshot-${name}`}
                  value={typeof options.snapshot === "string" ? options.snapshot : ""}
                  onChange={(event) => setOption("snapshot", event.target.value)}
                  placeholder="Provider default"
                  disabled={disabled}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor={`sandbox-workspace-${name}`}>Workspace path</Label>
                <Input
                  id={`sandbox-workspace-${name}`}
                  value={typeof options.workspace_path === "string" ? options.workspace_path : ""}
                  onChange={(event) => setOption("workspace_path", event.target.value)}
                  placeholder="/home/daytona/workspace"
                  disabled={disabled}
                  className="font-mono"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor={`sandbox-auto-stop-${name}`}>Provider auto-stop (minutes)</Label>
                <Input
                  id={`sandbox-auto-stop-${name}`}
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
          )}
        </div>
      ) : null}

      <div className="grid gap-4 sm:grid-cols-2">
        <div className="space-y-2">
          <Label htmlFor={`sandbox-idle-action-${name}`}>When idle</Label>
          <Select
            value={lifecycle.idle_action}
            onValueChange={(idle_action) =>
              onChange({
                ...spec,
                lifecycle: {
                  ...lifecycle,
                  idle_action: idle_action as typeof lifecycle.idle_action,
                },
              })
            }
            disabled={disabled}
          >
            <SelectTrigger id={`sandbox-idle-action-${name}`} className="w-full">
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
          <Label htmlFor={`sandbox-idle-after-${name}`}>Idle after (seconds)</Label>
          <Input
            id={`sandbox-idle-after-${name}`}
            type="number"
            min={1}
            max={86_400}
            value={lifecycle.idle_after_seconds}
            onChange={(event) =>
              onChange({
                ...spec,
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
          <Label htmlFor={`sandbox-bootstrap-${name}`}>Bootstrap commands</Label>
          <Textarea
            id={`sandbox-bootstrap-${name}`}
            value={(spec.bootstrap?.commands ?? []).join("\n")}
            onChange={(event) =>
              onChange({
                ...spec,
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

export function SandboxPolicyEditor({
  value,
  onChange,
  disabled = false,
  definitionMode = false,
}: {
  value: SandboxPolicy | null;
  onChange: (value: SandboxPolicy | null) => void;
  disabled?: boolean;
  /** Edit one reusable Sandbox Template definition, not an Agent binding policy. */
  definitionMode?: boolean;
}) {
  const { data, isLoading, error } = useSandboxTargets();
  const { data: sandboxTemplates = [] } = useSandboxTemplates();
  const targets = useMemo(() => data?.items ?? [], [data?.items]);
  const templates = value?.templates ?? {};
  const entries = Object.entries(templates);

  const add = (target: SandboxTargetDescriptor) => {
    const name = nextSandboxBindingName(target, templates);
    onChange({
      mode: value?.mode ?? "fixed",
      default: value?.default || name,
      templates: { ...templates, [name]: createSandboxTemplateSpec(target) },
    });
  };

  const addReusable = (sandboxTemplate: (typeof sandboxTemplates)[number]) => {
    const base = sandboxTemplate.name;
    const name = templates[base]
      ? nextSandboxBindingName(
          sandboxTemplate.current_revision.spec.target as SandboxTargetDescriptor,
          templates,
        )
      : base;
    onChange({
      mode: value?.mode ?? "fixed",
      default: value?.default || name,
      templates: {
        ...templates,
        [name]: {
          ...sandboxTemplate.current_revision.spec,
          template_revision_id: sandboxTemplate.current_revision.id,
        },
      },
    });
  };

  const update = (name: string, spec: SandboxTemplateSpec) => {
    onChange({
      mode: value?.mode ?? "fixed",
      default: value?.default || name,
      templates: { ...templates, [name]: spec },
    });
  };

  const rename = (from: string, to: string) => {
    const renamed: Record<string, SandboxTemplateSpec> = {};
    for (const [name, spec] of Object.entries(templates)) {
      renamed[name === from ? to : name] = spec;
    }
    onChange({
      mode: value?.mode ?? "fixed",
      default: value?.default === from ? to : value?.default || to,
      templates: renamed,
    });
  };

  const remove = (name: string) => {
    const remaining = Object.fromEntries(
      Object.entries(templates).filter(([templateName]) => templateName !== name),
    );
    const names = Object.keys(remaining);
    if (names.length === 0) {
      onChange(null);
      return;
    }
    onChange({
      mode: value?.mode ?? "fixed",
      default: value?.default === name ? names[0] : value?.default || names[0],
      templates: remaining,
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
              New Playground sessions pin one named sandbox binding. Checkpointed sandboxes restore
              the workspace into replacement compute if the physical sandbox disappears; running
              processes do not survive replacement.
            </p>
          </div>
        </div>
      </div>

      {!definitionMode ? (
        <div className="space-y-2">
          <Label htmlFor="sandbox-policy">Session choice</Label>
          <Select
            value={value?.mode ?? "fixed"}
            onValueChange={(mode) =>
              onChange(value ? { ...value, mode: mode as SandboxPolicy["mode"] } : value)
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
              <SelectItem value="selectable">Selectable — declared sandboxes only</SelectItem>
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

      {entries.map(([name, spec]) => (
        <TemplateSpecEditor
          key={name}
          name={name}
          spec={spec}
          templates={templates}
          isDefault={value?.default === name}
          targets={targets}
          disabled={disabled}
          onChange={(next) => update(name, { ...next, template_revision_id: undefined })}
          onRename={(next) => rename(name, next)}
          onDefault={() => onChange({ mode: value?.mode ?? "fixed", default: name, templates })}
          onRemove={() => remove(name)}
        />
      ))}

      {error ? (
        <p className="flex items-start gap-1.5 text-sm text-destructive">
          <TriangleAlert className="mt-0.5 size-4 shrink-0" />
          Could not load the execution targets offered by this deployment.
        </p>
      ) : null}

      {!disabled && entries.length < MAX_TEMPLATES ? (
        <div className="space-y-2">
          {!definitionMode && sandboxTemplates.length > 0 ? (
            <>
              <p className="text-xs font-medium text-muted-foreground">Use a Sandbox Template</p>
              <div className="flex flex-wrap gap-2">
                {sandboxTemplates.map((sandboxTemplate) => (
                  <Button
                    key={sandboxTemplate.id}
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() => addReusable(sandboxTemplate)}
                    disabled={(value?.mode ?? "fixed") === "fixed" && entries.length > 0}
                  >
                    <Box className="size-4" />
                    <Plus className="size-3" />
                    {sandboxTemplate.display_name}
                  </Button>
                ))}
              </div>
              <Link href="/sandbox-templates" className="text-xs text-muted-foreground underline">
                Manage Sandbox Templates
              </Link>
            </>
          ) : null}
          <p className="text-xs font-medium text-muted-foreground">Add a sandbox target</p>
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
                disabled={(value?.mode ?? "fixed") === "fixed" && entries.length > 0}
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
