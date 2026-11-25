"use client";

// Settings > Personal > Plugins: the organization's approved plugins a person
// adds to their own conversations. They apply in Work Chat wherever the
// channel lets people add plugins, the same switches as Work Chat's
// Customizations page. A plugin's skills and MCP servers are not listed on
// their own: a plugin is one row. Design: knowledge/integrations/plugins.md.

import { useUiTranslations } from "@/providers/locale-provider";
import { Package } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { EmptyState, IconTile } from "@/components/layout";
import { usePageTitle } from "@/hooks";
import { useMyPlugins, useSetMyPlugin } from "@/hooks/use-plugins";
import { pluginEnvironmentLabel } from "@/lib/api/plugins";

export default function MyPluginsSettingsPage() {
  const ui = useUiTranslations();
  usePageTitle(ui("Plugins"), ui("Settings"));
  const { data: plugins, isLoading, error } = useMyPlugins();
  const setPlugin = useSetMyPlugin();

  return (
    <div className="space-y-6">
      <div className="space-y-1">
        <h2 className="text-xl font-semibold">{ui("Plugins")}</h2>
        <p className="text-sm text-muted-foreground">
          {ui(
            "Plugins your organization approved that you turn on for yourself. They join your new Work Chat conversations wherever the chat lets people add plugins.",
          )}
        </p>
      </div>
      {isLoading ? (
        <Skeleton className="h-32 w-full" />
      ) : error ? (
        <p className="text-sm text-destructive">{error.message}</p>
      ) : plugins && plugins.length > 0 ? (
        <div className="border bg-card">
          {plugins.map((plugin) => {
            const title = plugin.display_name ?? plugin.name;
            const on = plugin.origin === "added" && plugin.enabled;
            return (
              <div
                key={plugin.id}
                className="flex items-center justify-between gap-4 border-t p-4 first:border-t-0"
              >
                <div className="flex min-w-0 items-start gap-3">
                  <IconTile size="md" icon={<Package />} />
                  <div className="min-w-0 space-y-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="font-medium">{title}</span>
                      <Badge variant="secondary">
                        {ui(pluginEnvironmentLabel(plugin.environment))}
                      </Badge>
                      {plugin.needs_admin_setup && (
                        <Badge variant="outline" className="border-warning text-warning">
                          {ui("Waiting for an admin")}
                        </Badge>
                      )}
                    </div>
                    {plugin.description && (
                      <p className="line-clamp-2 text-sm text-muted-foreground">
                        {plugin.description}
                      </p>
                    )}
                  </div>
                </div>
                <Switch
                  checked={on}
                  disabled={setPlugin.isPending || (!on && plugin.needs_admin_setup)}
                  aria-label={
                    on
                      ? ui("Remove {value0}", { value0: title })
                      : ui("Add {value0}", { value0: title })
                  }
                  onCheckedChange={(enabled) => setPlugin.mutate({ id: plugin.id, enabled })}
                />
              </div>
            );
          })}
        </div>
      ) : (
        <EmptyState
          icon={<Package />}
          title={ui("No approved plugins")}
          description={ui("An organization admin approves plugins in Settings › Plugins.")}
        />
      )}
      {setPlugin.error && <p className="text-sm text-destructive">{setPlugin.error.message}</p>}
    </div>
  );
}
