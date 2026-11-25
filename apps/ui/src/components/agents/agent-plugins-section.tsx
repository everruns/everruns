"use client";

// Agent > More > Plugins: which of the organization's approved plugins this
// agent uses. "All approved plugins" (the default for new choices) stores one
// ref, `plugin:_all`, that the server expands when it builds the agent, so a
// plugin approved later reaches the agent without an edit. "Only the ones I
// pick" stores a `plugin:{id}` ref per plugin. "Load others when asked" is the
// `plugins` capability: the agent can search the approved list and load one
// into a chat when someone asks. Design: knowledge/integrations/plugins.md.

import { useUiTranslations } from "@/providers/locale-provider";
import { Badge } from "@/components/ui/badge";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { LinkButton } from "@/components/ui/button";
import { useInstalledPlugins } from "@/hooks/use-plugins";
import {
  ALL_APPROVED_PLUGINS_REF,
  environmentSatisfied,
  pluginEnvironmentLabel,
  providedEnvironment,
} from "@/lib/api/plugins";
import type { AgentCapabilityConfig } from "@/lib/api/types";
import { cn } from "@/lib/utils";

const PLUGINS_CAPABILITY = "plugins";

export function isPluginRef(ref: string): boolean {
  return ref.startsWith("plugin:");
}

/** The More row's summary for these refs. */
export function agentPluginsSummary(
  capabilities: readonly AgentCapabilityConfig[],
  approvedCount: number | undefined,
  ui: (text: string, values?: Record<string, string | number>) => string,
): string {
  const refs = capabilities.map((capability) => capability.ref);
  const loads = refs.includes(PLUGINS_CAPABILITY);
  const base = refs.includes(ALL_APPROVED_PLUGINS_REF)
    ? approvedCount === undefined
      ? ui("All approved")
      : ui("All approved ({value0})", { value0: approvedCount })
    : (() => {
        const picked = refs.filter((ref) => isPluginRef(ref)).length;
        return picked ? ui("{value0} picked", { value0: picked }) : ui("None");
      })();
  return loads ? `${base} · ${ui("load when asked")}` : base;
}

export function AgentPluginsSection({
  capabilities,
  inheritedRefs,
  onChange,
  readOnly,
}: {
  capabilities: AgentCapabilityConfig[];
  /** Capability refs the agent's harness chain contributes. */
  inheritedRefs: readonly string[];
  onChange: (next: AgentCapabilityConfig[]) => void;
  readOnly: boolean;
}) {
  const ui = useUiTranslations();
  const { data: installed, isLoading } = useInstalledPlugins();
  const approved = (installed ?? []).filter((plugin) => plugin.status === "active");
  const refs = capabilities.map((capability) => capability.ref);
  const all = refs.includes(ALL_APPROVED_PLUGINS_REF);
  const loads = refs.includes(PLUGINS_CAPABILITY);
  const provided = providedEnvironment([...inheritedRefs, ...refs]);
  const others = capabilities.filter((capability) => !isPluginRef(capability.ref));

  const setMode = (mode: "all" | "pick") => {
    if (mode === "all") {
      onChange([...others, { ref: ALL_APPROVED_PLUGINS_REF, config: {} }]);
    } else {
      // Start from what "all" gave the agent, so switching changes nothing yet.
      const current = approved
        .filter(
          (plugin) =>
            plugin.identity_required.length === 0 &&
            environmentSatisfied(plugin.environment, provided),
        )
        .map((plugin) => ({ ref: plugin.capability_ref, config: {} }));
      onChange([...others, ...current]);
    }
  };
  const togglePick = (ref: string, on: boolean) => {
    const rest = capabilities.filter((capability) => capability.ref !== ref);
    onChange(on ? [...rest, { ref, config: {} }] : rest);
  };
  const setLoads = (on: boolean) => {
    const rest = capabilities.filter((capability) => capability.ref !== PLUGINS_CAPABILITY);
    onChange(on ? [...rest, { ref: PLUGINS_CAPABILITY, config: {} }] : rest);
  };

  const option = (mode: "all" | "pick", title: string, description: string) => {
    const selected = mode === "all" ? all : !all;
    return (
      <button
        type="button"
        disabled={readOnly}
        onClick={() => setMode(mode)}
        aria-pressed={selected}
        className={cn(
          "flex w-full items-start gap-3 border p-3 text-left",
          selected ? "border-primary bg-muted/40" : "hover:bg-muted/30",
        )}
      >
        <span
          className={cn(
            "mt-1 size-3 shrink-0 rounded-full border",
            selected && "border-primary bg-primary",
          )}
        />
        <span className="space-y-0.5">
          <span className="block text-sm font-medium">{title}</span>
          <span className="block text-xs text-muted-foreground">{description}</span>
        </span>
      </button>
    );
  };

  return (
    <div className="space-y-5">
      <div className="space-y-2">
        {option(
          "all",
          ui("All approved plugins"),
          ui("Every plugin the organization approves, now and later, that this agent can run."),
        )}
        {option("pick", ui("Only the ones I pick"), ui("Choose from the approved list."))}
      </div>

      {isLoading ? (
        <Skeleton className="h-24 w-full" />
      ) : approved.length === 0 ? (
        <div className="space-y-2 border p-4 text-sm text-muted-foreground">
          <p>{ui("Your organization has not approved any plugins yet.")}</p>
          <LinkButton href="/settings/plugins" variant="outline" size="sm">
            {ui("Open Settings › Plugins")}
          </LinkButton>
        </div>
      ) : (
        <div className="border">
          {approved.map((plugin) => {
            const fits = environmentSatisfied(plugin.environment, provided);
            const needsSetup = plugin.identity_required.length > 0;
            const included = all ? fits && !needsSetup : refs.includes(plugin.capability_ref);
            return (
              <div
                key={plugin.id}
                className="flex items-center justify-between gap-3 border-t p-3 first:border-t-0"
              >
                <div className="min-w-0">
                  <div className="text-sm font-medium">{plugin.display_name ?? plugin.name}</div>
                  <div className="text-xs text-muted-foreground">
                    {fits
                      ? ui(pluginEnvironmentLabel(plugin.environment))
                      : ui("Needs {value0}; this agent is {value1}", {
                          value0: ui(pluginEnvironmentLabel(plugin.environment)),
                          value1: ui(pluginEnvironmentLabel(provided)),
                        })}
                  </div>
                </div>
                {all ? (
                  <Badge variant={included ? "success" : "outline"}>
                    {included
                      ? ui("Included")
                      : needsSetup
                        ? ui("Waiting for an admin")
                        : ui("Can't run here")}
                  </Badge>
                ) : (
                  <Switch
                    checked={included}
                    disabled={readOnly || (!included && (!fits || needsSetup))}
                    aria-label={plugin.display_name ?? plugin.name}
                    onCheckedChange={(on) => togglePick(plugin.capability_ref, on)}
                  />
                )}
              </div>
            );
          })}
        </div>
      )}

      <div className="flex items-start gap-3">
        <Switch
          id="agent-plugins-load"
          checked={loads}
          disabled={readOnly}
          onCheckedChange={setLoads}
        />
        <div className="space-y-0.5">
          <Label htmlFor="agent-plugins-load" className="text-sm font-normal">
            {ui("Load others when asked")}
          </Label>
          <p className="text-xs text-muted-foreground">
            {ui(
              "When someone asks for a service this agent does not have, it can find an approved plugin and add it to that chat.",
            )}
          </p>
        </div>
      </div>
    </div>
  );
}
