"use client";

// Settings > Organization > Plugins: the plugins agents and people in this
// organization may use. Approving a directory plugin installs it; the switch
// pauses it everywhere without uninstalling it. Agents take every approved
// plugin unless set otherwise, and people pick from this list for themselves.
// Design: knowledge/integrations/plugins.md ("UI"). Marketplaces are an
// admin's "Sources", opened from here.

import { useUiTranslations } from "@/providers/locale-provider";
import { useMemo, useState } from "react";
import { Ellipsis, Package, RefreshCw, Store, Trash2 } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuPositioner,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { SearchInput } from "@/components/ui/search-input";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { EmptyState, IconTile } from "@/components/layout";
import { usePageTitle } from "@/hooks";
import { usePolicies } from "@/hooks/use-policies";
import {
  useInstallPlugin,
  useInstalledPlugins,
  useMarketplaceCatalog,
  useMarketplaces,
  usePatchInstalledPlugin,
  useUpdateInstalledPlugin,
} from "@/hooks/use-plugins";
import { pluginEnvironmentLabel } from "@/lib/api/plugins";
import type { InstalledPlugin, Marketplace } from "@/lib/api/types";
import {
  AddMarketplaceDialog,
  MarketplaceCard,
  RemoveMarketplaceDialog,
  UninstallPluginDialog,
} from "@/components/plugins/plugin-sources";

/** The default source is the Everruns Plugin directory. */
function sourceTitle(marketplace: Marketplace): string {
  return marketplace.name === "everruns" ? "Plugin directory" : marketplace.name;
}

function ApprovedPluginRow({
  plugin,
  canManage,
  onChooseSignIn,
  onRemove,
}: {
  plugin: InstalledPlugin;
  canManage: boolean;
  onChooseSignIn: (plugin: InstalledPlugin) => void;
  onRemove: (plugin: InstalledPlugin) => void;
}) {
  const ui = useUiTranslations();
  const patch = usePatchInstalledPlugin(plugin.id);
  const update = useUpdateInstalledPlugin();
  const enabled = plugin.status === "active";
  const needsSignInChoice = plugin.identity_required.length > 0;
  const title = plugin.display_name ?? plugin.name;

  return (
    <div className="flex items-center justify-between gap-4 border-t p-4 first:border-t-0">
      <div className="flex min-w-0 items-start gap-3">
        <IconTile size="md" icon={<Package />} />
        <div className="min-w-0 space-y-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className={enabled ? "font-medium" : "font-medium text-muted-foreground"}>
              {title}
            </span>
            {plugin.version && <Badge variant="outline">{plugin.version}</Badge>}
            <Badge variant="secondary">{ui(pluginEnvironmentLabel(plugin.environment))}</Badge>
            {needsSignInChoice && (
              <Badge variant="outline" className="border-warning text-warning">
                {ui("Choose sign-in")}
              </Badge>
            )}
            {plugin.update_available && <Badge variant="outline">{ui("Update available")}</Badge>}
          </div>
          {plugin.description && (
            <p className="line-clamp-2 text-sm text-muted-foreground">{plugin.description}</p>
          )}
          {patch.error && <p className="text-sm text-destructive">{patch.error.message}</p>}
        </div>
      </div>
      {canManage && (
        <div className="flex shrink-0 items-center gap-2">
          {needsSignInChoice ? (
            <Button variant="outline" size="sm" onClick={() => onChooseSignIn(plugin)}>
              {ui("Choose")}
            </Button>
          ) : (
            <Switch
              checked={enabled}
              disabled={patch.isPending}
              aria-label={
                enabled
                  ? ui("Pause {value0}", { value0: title })
                  : ui("Turn on {value0}", { value0: title })
              }
              onCheckedChange={(next) => patch.mutate({ status: next ? "active" : "disabled" })}
            />
          )}
          <DropdownMenu>
            <DropdownMenuTrigger
              render={<Button variant="ghost" size="icon-sm" />}
              aria-label={ui("More actions for {value0}", { value0: title })}
            >
              <Ellipsis />
            </DropdownMenuTrigger>
            <DropdownMenuPositioner align="end">
              <DropdownMenuContent>
                {plugin.update_available && (
                  <DropdownMenuItem
                    onClick={() => update.mutate(plugin.id)}
                    disabled={update.isPending}
                  >
                    <RefreshCw />
                    {ui("Update")}
                  </DropdownMenuItem>
                )}
                {needsSignInChoice || (
                  <DropdownMenuItem onClick={() => onChooseSignIn(plugin)}>
                    {ui("How it signs in")}
                  </DropdownMenuItem>
                )}
                <DropdownMenuItem variant="destructive" onClick={() => onRemove(plugin)}>
                  <Trash2 />
                  {ui("Remove")}
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenuPositioner>
          </DropdownMenu>
        </div>
      )}
    </div>
  );
}

/** Pick, per MCP server, whether it acts as each person or as the agent. */
function SignInChoiceDialog({
  plugin,
  onOpenChange,
}: {
  plugin: InstalledPlugin | null;
  onOpenChange: (open: boolean) => void;
}) {
  const ui = useUiTranslations();
  const patch = usePatchInstalledPlugin(plugin?.id ?? "");
  const servers = plugin?.identity_required ?? [];
  const choose = (server: string, actsAs: "user" | "service") =>
    patch.mutate({ mcp_server_identities: { [server]: actsAs } });

  return (
    <Dialog open={plugin !== null} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>
            {ui("How {value0} signs in", { value0: plugin?.display_name ?? plugin?.name ?? "" })}
          </DialogTitle>
          <DialogDescription>
            {servers.length > 0
              ? ui(
                  "Each person can sign in with their own account, or the agent can use one shared account. The plugin turns on once every server has a choice.",
                )
              : ui("Every server of this plugin already has a choice.")}
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-3">
          {servers.map((server) => (
            <div key={server} className="flex flex-wrap items-center justify-between gap-2">
              <code className="text-xs">{server}</code>
              <div className="flex gap-2">
                <Button
                  variant="outline"
                  size="sm"
                  disabled={patch.isPending}
                  onClick={() => choose(server, "user")}
                >
                  {ui("Each person signs in")}
                </Button>
                <Button
                  variant="outline"
                  size="sm"
                  disabled={patch.isPending}
                  onClick={() => choose(server, "service")}
                >
                  {ui("The agent's account")}
                </Button>
              </div>
            </div>
          ))}
          {patch.error && <p className="text-sm text-destructive">{patch.error.message}</p>}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {ui("Done")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function DirectoryTiles({
  marketplace,
  search,
  canManage,
}: {
  marketplace: Marketplace;
  search: string;
  canManage: boolean;
}) {
  const ui = useUiTranslations();
  const { data: catalog, isLoading, error } = useMarketplaceCatalog(marketplace.id);
  const install = useInstallPlugin(marketplace.id, marketplace.name);
  const query = search.trim().toLowerCase();
  const entries = (catalog ?? []).filter(
    (entry) =>
      !entry.installed &&
      (!query ||
        [entry.name, entry.display_name, entry.description, entry.category]
          .filter(Boolean)
          .some((text) => text!.toLowerCase().includes(query))),
  );

  if (isLoading) {
    return <Skeleton className="h-24 w-full" />;
  }
  // A source that has never synced, or whose last sync failed, has nothing to
  // list; saying "everything is approved" there would be wrong.
  if (error || !marketplace.last_synced_at) {
    return (
      <p className="text-sm text-muted-foreground">
        {!marketplace.last_synced_at
          ? ui("{value0} has not synced yet. Sync it in Sources.", {
              value0: sourceTitle(marketplace),
            })
          : ui("Could not read {value0}: {value1}", {
              value0: sourceTitle(marketplace),
              value1: error?.message ?? "",
            })}
      </p>
    );
  }
  if (entries.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        {query
          ? ui("Nothing in {value0} matches.", { value0: sourceTitle(marketplace) })
          : ui("Everything in {value0} is approved.", { value0: sourceTitle(marketplace) })}
      </p>
    );
  }
  return (
    <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
      {entries.map((entry) => {
        const pending = install.isPending && install.variables?.plugin_name === entry.name;
        return (
          <Card key={entry.name} className="flex flex-col gap-2 p-4">
            <div className="flex items-center gap-2">
              <span className="font-medium">{entry.display_name ?? entry.name}</span>
              {entry.category && <Badge variant="outline">{entry.category}</Badge>}
            </div>
            {entry.description && (
              <p className="line-clamp-2 text-sm text-muted-foreground">{entry.description}</p>
            )}
            <div className="mt-auto flex items-center justify-between gap-2 pt-1">
              <span className="truncate text-xs text-muted-foreground">
                {entry.author ?? entry.version ?? ""}
              </span>
              {canManage && (
                <Button
                  variant="accent"
                  size="sm"
                  disabled={install.isPending}
                  onClick={() =>
                    install.mutate({ marketplace_id: marketplace.id, plugin_name: entry.name })
                  }
                >
                  {pending ? ui("Approving...") : ui("Approve")}
                </Button>
              )}
            </div>
          </Card>
        );
      })}
      {install.error && (
        <p className="text-sm text-destructive md:col-span-2 xl:col-span-3">
          {install.error.message}
        </p>
      )}
    </div>
  );
}

function SourcesDialog({
  open,
  onOpenChange,
  marketplaces,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  marketplaces: Marketplace[];
}) {
  const ui = useUiTranslations();
  const [adding, setAdding] = useState(false);
  const [removing, setRemoving] = useState<Marketplace | null>(null);
  return (
    <>
      <Dialog open={open} onOpenChange={onOpenChange}>
        <DialogContent className="sm:max-w-3xl">
          <DialogHeader>
            <DialogTitle>{ui("Sources")}</DialogTitle>
            <DialogDescription>
              {ui(
                "Catalogs plugins are approved from. Every organization starts with the Everruns Plugin directory; add another marketplace by GitHub repository or URL.",
              )}
            </DialogDescription>
          </DialogHeader>
          <div className="grid max-h-[60vh] gap-3 overflow-y-auto md:grid-cols-2">
            {marketplaces.map((marketplace) => (
              <MarketplaceCard
                key={marketplace.id}
                marketplace={marketplace}
                onRemove={setRemoving}
                onBrowse={() => onOpenChange(false)}
              />
            ))}
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setAdding(true)}>
              <Store className="size-4" />
              {ui("Add source")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <AddMarketplaceDialog open={adding} onOpenChange={setAdding} />
      <RemoveMarketplaceDialog
        marketplace={removing}
        open={removing !== null}
        onOpenChange={(next) => !next && setRemoving(null)}
      />
    </>
  );
}

export default function PluginsSettingsPage() {
  const ui = useUiTranslations();
  usePageTitle(ui("Plugins"), ui("Settings"));
  const policies = usePolicies("plugins");
  const canManage = policies.can("plugin.manage");
  const { data: plugins, isLoading: pluginsLoading, error } = useInstalledPlugins();
  const { data: marketplaces } = useMarketplaces();
  const [search, setSearch] = useState("");
  const [sourcesOpen, setSourcesOpen] = useState(false);
  const [signIn, setSignIn] = useState<InstalledPlugin | null>(null);
  const [removing, setRemoving] = useState<InstalledPlugin | null>(null);
  const activeSources = useMemo(
    () => (marketplaces ?? []).filter((marketplace) => marketplace.status === "active"),
    [marketplaces],
  );
  // The dialog reads the live row, so a choice shows at once.
  const signInPlugin = signIn ? (plugins?.find((p) => p.id === signIn.id) ?? signIn) : null;

  return (
    <div className="space-y-8">
      <div className="space-y-1">
        <h2 className="text-xl font-semibold">{ui("Plugins")}</h2>
        <p className="text-sm text-muted-foreground">
          {ui(
            "Plugins agents and people in this organization may use: skills and MCP servers for services, packaged together. Agents use every approved plugin unless set otherwise, and people can add approved plugins for themselves.",
          )}
        </p>
      </div>

      <section className="space-y-3">
        <h3 className="text-sm font-medium text-muted-foreground">
          {ui("Approved")}
          {plugins ? ` · ${plugins.length}` : ""}
        </h3>
        {pluginsLoading ? (
          <Skeleton className="h-32 w-full" />
        ) : error ? (
          <p className="text-sm text-destructive">{error.message}</p>
        ) : plugins && plugins.length > 0 ? (
          <div className="border bg-card">
            {plugins.map((plugin) => (
              <ApprovedPluginRow
                key={plugin.id}
                plugin={plugin}
                canManage={canManage}
                onChooseSignIn={setSignIn}
                onRemove={setRemoving}
              />
            ))}
          </div>
        ) : (
          <EmptyState
            icon={<Package />}
            title={ui("No approved plugins yet")}
            description={ui("Approve one from the directory below.")}
          />
        )}
      </section>

      <section className="space-y-4">
        <div className="flex flex-wrap items-center gap-3">
          <h3 className="mr-auto text-sm font-medium text-muted-foreground">
            {ui("Available to approve")}
          </h3>
          <SearchInput
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder={ui("Search plugins")}
            containerClassName="w-64"
          />
          {canManage && (
            <Button variant="outline" onClick={() => setSourcesOpen(true)}>
              {ui("Sources")}
            </Button>
          )}
        </div>
        {activeSources.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {ui("No sources are on. Turn one on in Sources to approve plugins from it.")}
          </p>
        ) : (
          activeSources.map((marketplace) => (
            <div key={marketplace.id} className="space-y-2">
              {activeSources.length > 1 && (
                <p className="text-xs font-medium text-muted-foreground">
                  {ui(sourceTitle(marketplace))}
                </p>
              )}
              <DirectoryTiles marketplace={marketplace} search={search} canManage={canManage} />
            </div>
          ))
        )}
      </section>

      <SourcesDialog
        open={sourcesOpen}
        onOpenChange={setSourcesOpen}
        marketplaces={marketplaces ?? []}
      />
      <SignInChoiceDialog plugin={signInPlugin} onOpenChange={(next) => !next && setSignIn(null)} />
      <UninstallPluginDialog
        plugin={removing}
        open={removing !== null}
        onOpenChange={(next) => !next && setRemoving(null)}
      />
    </div>
  );
}
