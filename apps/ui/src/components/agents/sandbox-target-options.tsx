"use client";

/**
 * Per-target option fields of the sandbox template editor: Modal, E2B, boxd,
 * Isolated sandbox, and the network policy shared by all of them.
 */

import { useUiTranslations } from "@/providers/locale-provider";
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
import type { SandboxTemplateSpec, SandboxTargetDescriptor } from "@/lib/api/types";

/** Sizes of the Isolated sandbox target (kubernetes_sandbox::SandboxSize). */
const ISOLATED_SIZES: Record<string, string> = {
  small: "Small · 1 vCPU · 2 GiB",
  medium: "Medium · 2 vCPU · 4 GiB",
  large: "Large · 4 vCPU · 8 GiB",
};

export function optionString(options: Record<string, unknown>, key: string): string {
  const value = options[key];
  return typeof value === "string" || typeof value === "number" ? String(value) : "";
}

/** Modal fields; the server and provider validate the same ranges. */
export type NetworkPolicy = NonNullable<NonNullable<SandboxTemplateSpec["containment"]>["network"]>;

/**
 * Outbound network policy for providers that enforce it themselves (Modal,
 * Daytona, E2B, boxd). Renders grid cells, so it sits inside a two-column grid.
 */
export function NetworkPolicyFields({
  name,
  network,
  disabled,
  onNetworkChange,
  ipRangesOnly = false,
}: {
  name: string;
  network: NetworkPolicy;
  disabled: boolean;
  onNetworkChange: (network: NetworkPolicy) => void;
  /** The target enforces IP ranges but cannot match domain names. */
  ipRangesOnly?: boolean;
}) {
  const ui = useUiTranslations();
  return (
    <>
      <div className="space-y-2">
        <Label htmlFor={`sandbox-network-${name}`}>{ui("Outbound network")}</Label>
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
            <SelectItem value="allow">{ui("Open")}</SelectItem>
            <SelectItem value="allowlist">
              {ipRangesOnly ? ui("Only listed IP ranges") : ui("Only listed domains")}
            </SelectItem>
            <SelectItem value="deny">{ui("Blocked")}</SelectItem>
          </SelectContent>
        </Select>
      </div>
      {network.mode === "allowlist" ? (
        <div className="space-y-2 sm:col-span-2">
          <Label htmlFor={`sandbox-domains-${name}`}>
            {ipRangesOnly ? ui("Allowed IP ranges") : ui("Allowed domains")}
          </Label>
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
            placeholder={
              ipRangesOnly ? "10.0.0.0/8\n203.0.113.7/32" : "pypi.org\n*.pythonhosted.org"
            }
            disabled={disabled}
            className="font-mono"
          />
          {ipRangesOnly ? (
            <p className="text-xs text-muted-foreground">
              {ui(
                "Enforced by the cluster network, so domain names cannot be matched here. Use Open for name-based rules at the egress proxy.",
              )}
            </p>
          ) : null}
        </div>
      ) : null}
    </>
  );
}

export function ModalOptionFields({
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
  const ui = useUiTranslations();
  const injected = Array.isArray(options.inject_connections) ? options.inject_connections : [];
  const injectGithub = injected.includes("github");
  // Modal cannot inject credentials when egress is blocked or limited to domains.
  const injectionAllowed = network.mode === "allow";
  const numeric = (key: string) => (event: React.ChangeEvent<HTMLInputElement>) =>
    setOption(key, event.target.value ? Number(event.target.value) : "");
  return (
    <div className="grid gap-4 sm:grid-cols-2">
      <div className="space-y-2">
        <Label htmlFor={`sandbox-runtime-${name}`}>{ui("Runtime")}</Label>
        <Select
          value={typeof options.runtime === "string" ? options.runtime : "vm"}
          onValueChange={(value) => setOption("runtime", value === "vm" ? "" : value)}
          disabled={disabled}
        >
          <SelectTrigger id={`sandbox-runtime-${name}`} className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="vm">{ui("VM (own kernel)")}</SelectItem>
            <SelectItem value="gvisor">{ui("gVisor container")}</SelectItem>
          </SelectContent>
        </Select>
      </div>
      <div className="space-y-2">
        <Label htmlFor={`sandbox-image-${name}`}>{ui("Image")}</Label>
        <Input
          id={`sandbox-image-${name}`}
          value={optionString(options, "image")}
          onChange={(event) => setOption("image", event.target.value)}
          placeholder={ui("python:3.13-slim with git and curl")}
          disabled={disabled}
          className="font-mono"
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor={`sandbox-cpu-${name}`}>{ui("CPU cores")}</Label>
        <Input
          id={`sandbox-cpu-${name}`}
          type="number"
          min={0.125}
          max={64}
          step={0.125}
          value={optionString(options, "cpu")}
          onChange={numeric("cpu")}
          placeholder={ui("Provider default")}
          disabled={disabled}
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor={`sandbox-memory-${name}`}>{ui("Memory (MiB)")}</Label>
        <Input
          id={`sandbox-memory-${name}`}
          type="number"
          min={128}
          max={262_144}
          value={optionString(options, "memory_mb")}
          onChange={numeric("memory_mb")}
          placeholder={ui("Provider default")}
          disabled={disabled}
        />
      </div>
      <div className="space-y-2 sm:col-span-2">
        <Label htmlFor={`sandbox-workspace-${name}`}>{ui("Workspace path")}</Label>
        <Input
          id={`sandbox-workspace-${name}`}
          value={optionString(options, "workspace_path")}
          onChange={(event) => setOption("workspace_path", event.target.value)}
          placeholder="/workspace"
          disabled={disabled}
          className="font-mono"
        />
      </div>
      <div className="flex items-start gap-3 pt-6">
        <Switch
          id={`sandbox-inject-github-${name}`}
          aria-label={ui("Use my GitHub connection")}
          checked={injectGithub && injectionAllowed}
          onCheckedChange={(checked) => setOption("inject_connections", checked ? ["github"] : "")}
          disabled={disabled || !injectionAllowed}
        />
        <p className="text-xs text-muted-foreground">
          {ui(
            "Use my GitHub connection for api.github.com and git. Modal adds the token to requests, so the agent never sees it.",
          )}
          {injectionAllowed ? "" : ui(" Needs open network.")}
        </p>
      </div>
      <NetworkPolicyFields
        name={name}
        network={network}
        disabled={disabled}
        onNetworkChange={onNetworkChange}
      />
    </div>
  );
}

export function DesktopField({
  name,
  options,
  disabled,
  onDesktopChange,
}: {
  name: string;
  options: Record<string, unknown>;
  disabled: boolean;
  onDesktopChange: (on: boolean) => void;
}) {
  const ui = useUiTranslations();
  const on =
    options.desktop === true || (typeof options.desktop === "object" && options.desktop !== null);
  return (
    <div className="flex items-start gap-3 sm:col-span-2">
      <Switch
        id={`sandbox-desktop-${name}`}
        aria-label={ui("Desktop")}
        checked={on}
        onCheckedChange={onDesktopChange}
        disabled={disabled}
      />
      <div className="space-y-1">
        <Label htmlFor={`sandbox-desktop-${name}`}>{ui("Desktop")}</Label>
        <p className="text-xs text-muted-foreground">
          {ui(
            "Start the sandbox with a Linux desktop and give the agent the computer tool to see and use it. Screenshots cost image tokens.",
          )}
        </p>
      </div>
    </div>
  );
}

export function E2BOptionFields({
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
  const ui = useUiTranslations();
  return (
    <div className="grid gap-4 sm:grid-cols-2">
      <div className="space-y-2">
        <Label htmlFor={`sandbox-template-${name}`}>{ui("E2B template")}</Label>
        <Input
          id={`sandbox-template-${name}`}
          value={optionString(options, "template")}
          onChange={(event) => setOption("template", event.target.value)}
          placeholder={ui("base")}
          disabled={disabled}
          className="font-mono"
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor={`sandbox-timeout-${name}`}>{ui("Provider timeout (seconds)")}</Label>
        <Input
          id={`sandbox-timeout-${name}`}
          type="number"
          min={1}
          max={86_400}
          value={optionString(options, "timeout_seconds")}
          onChange={(event) =>
            setOption("timeout_seconds", event.target.value ? Number(event.target.value) : "")
          }
          placeholder="3600"
          disabled={disabled}
        />
      </div>
      <div className="space-y-2 sm:col-span-2">
        <Label htmlFor={`sandbox-workspace-${name}`}>{ui("Workspace path")}</Label>
        <Input
          id={`sandbox-workspace-${name}`}
          value={optionString(options, "workspace_path")}
          onChange={(event) => setOption("workspace_path", event.target.value)}
          placeholder="/home/user"
          disabled={disabled}
          className="font-mono"
        />
      </div>
      <DesktopField
        name={name}
        options={options}
        disabled={disabled}
        onDesktopChange={(on) => setOption("desktop", on ? true : "")}
      />
      <NetworkPolicyFields
        name={name}
        network={network}
        disabled={disabled}
        onNetworkChange={onNetworkChange}
      />
    </div>
  );
}

/** boxd fields; the server checks them with the provider's own rules. */
export function BoxdOptionFields({
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
  const ui = useUiTranslations();
  // A machine from a snapshot keeps the snapshot's size, so the two fields
  // exclude each other: clear one to use the other.
  const fromSnapshot = optionString(options, "snapshot") !== "";
  const sized = optionString(options, "size") !== "";
  return (
    <div className="grid gap-4 sm:grid-cols-2">
      <div className="space-y-2">
        <Label htmlFor={`sandbox-region-${name}`}>{ui("Region")}</Label>
        <Select
          value={typeof options.region === "string" ? options.region : "default"}
          onValueChange={(value) => setOption("region", value === "default" ? "" : value)}
          disabled={disabled}
        >
          <SelectTrigger id={`sandbox-region-${name}`} className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="default">{ui("Provider default")}</SelectItem>
            <SelectItem value="fra">{ui("Frankfurt")}</SelectItem>
            <SelectItem value="nyc">{ui("New York")}</SelectItem>
          </SelectContent>
        </Select>
      </div>
      <div className="space-y-2">
        <Label htmlFor={`sandbox-size-${name}`}>{ui("Compute size")}</Label>
        <Select
          value={typeof options.size === "string" ? options.size : "default"}
          onValueChange={(value) => setOption("size", value === "default" ? "" : value)}
          disabled={disabled || fromSnapshot}
        >
          <SelectTrigger id={`sandbox-size-${name}`} className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="default">{ui("Provider default")}</SelectItem>
            <SelectItem value="1x4">1 vCPU, 4 GiB</SelectItem>
            <SelectItem value="2x8">2 vCPU, 8 GiB</SelectItem>
            <SelectItem value="4x16">4 vCPU, 16 GiB</SelectItem>
            <SelectItem value="8x32">8 vCPU, 32 GiB</SelectItem>
          </SelectContent>
        </Select>
      </div>
      <div className="space-y-2">
        <Label htmlFor={`sandbox-snapshot-${name}`}>{ui("Snapshot")}</Label>
        <Input
          id={`sandbox-snapshot-${name}`}
          value={optionString(options, "snapshot")}
          onChange={(event) => setOption("snapshot", event.target.value)}
          placeholder={ui("Provider default")}
          disabled={disabled || sized}
          className="font-mono"
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor={`sandbox-workspace-${name}`}>{ui("Workspace path")}</Label>
        <Input
          id={`sandbox-workspace-${name}`}
          value={optionString(options, "workspace_path")}
          onChange={(event) => setOption("workspace_path", event.target.value)}
          placeholder="/home/boxd/workspace"
          disabled={disabled}
          className="font-mono"
        />
      </div>
      <NetworkPolicyFields
        name={name}
        network={network}
        disabled={disabled}
        onNetworkChange={onNetworkChange}
      />
    </div>
  );
}

/**
 * Isolated sandbox fields. Sizes and the default image come from the
 * deployment (`isolation`); the server checks options with the provider's
 * own rules. Network enforcement is a NetworkPolicy, which cannot match
 * domain names, so the allowlist takes IP ranges only.
 */
export function IsolatedOptionFields({
  name,
  options,
  network,
  isolation,
  disabled,
  setOption,
  onNetworkChange,
}: {
  name: string;
  options: Record<string, unknown>;
  network: NetworkPolicy;
  isolation: SandboxTargetDescriptor["isolation"];
  disabled: boolean;
  setOption: (key: string, value: unknown) => void;
  onNetworkChange: (network: NetworkPolicy) => void;
}) {
  const ui = useUiTranslations();
  const offered = isolation?.sizes ?? ["small"];
  const notOffered = Object.keys(ISOLATED_SIZES).filter((size) => !offered.includes(size));
  return (
    <div className="grid gap-4 sm:grid-cols-2">
      <div className="space-y-2">
        <Label htmlFor={`sandbox-size-${name}`}>{ui("Size")}</Label>
        <Select
          value={typeof options.size === "string" ? options.size : "small"}
          onValueChange={(value) => setOption("size", value === "small" ? "" : value)}
          disabled={disabled}
        >
          <SelectTrigger id={`sandbox-size-${name}`} className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {offered.map((size) => (
              <SelectItem key={size} value={size}>
                {ui(ISOLATED_SIZES[size] ?? size)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {notOffered.length > 0 ? (
          <p className="text-xs text-muted-foreground">
            {ui("{value0} not offered on this deployment.", {
              value0: notOffered.map((size) => ISOLATED_SIZES[size]).join(", "),
            })}
          </p>
        ) : null}
      </div>
      <div className="space-y-2">
        <Label htmlFor={`sandbox-image-${name}`}>{ui("Image")}</Label>
        <Input
          id={`sandbox-image-${name}`}
          value={optionString(options, "image")}
          onChange={(event) => setOption("image", event.target.value)}
          placeholder={isolation?.default_image ?? "ghcr.io/everruns/sandbox-dev"}
          disabled={disabled}
          className="font-mono"
        />
        <p className="text-xs text-muted-foreground">
          {ui("Any image with the Everruns sandbox guest.")}{" "}
          <a
            href="https://everruns.com/docs/features/isolated-sandboxes/#images"
            target="_blank"
            rel="noreferrer"
            className="underline underline-offset-2"
          >
            {ui("How to build one")}
          </a>
        </p>
      </div>
      <div className="space-y-2 sm:col-span-2">
        <Label htmlFor={`sandbox-workspace-${name}`}>{ui("Workspace path")}</Label>
        <Input
          id={`sandbox-workspace-${name}`}
          value={optionString(options, "workspace_path")}
          onChange={(event) => setOption("workspace_path", event.target.value)}
          placeholder="/workspace"
          disabled={disabled}
          className="font-mono"
        />
        <p className="text-xs text-muted-foreground">
          {ui("Kept on its own volume while the sandbox is stopped.")}
        </p>
      </div>
      <DesktopField
        name={name}
        options={options}
        disabled={disabled}
        onDesktopChange={(on) => setOption("desktop", on ? true : "")}
      />
      <NetworkPolicyFields
        name={name}
        network={network}
        disabled={disabled}
        onNetworkChange={onNetworkChange}
        ipRangesOnly
      />
    </div>
  );
}
