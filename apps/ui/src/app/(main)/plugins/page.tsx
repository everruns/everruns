"use client";

import { useRef, useState } from "react";
import { inferMarketplaceSourceType, marketplaceSourceLabel } from "@/lib/api/plugins";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardActions,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuPositioner,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { EntityIdentity } from "@/components/ui/entity-identity";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  PageControlStrip,
  SectionTabs,
  PageMain,
  PageFooter,
} from "@/components/layout";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  useMarketplaces,
  useCreateMarketplace,
  useUpdateMarketplace,
  useSyncMarketplace,
  useDeleteMarketplace,
  useMarketplaceCatalog,
  useInstalledPlugins,
  useInstallPlugin,
  usePatchInstalledPlugin,
  useUpdateInstalledPlugin,
  useDeleteInstalledPlugin,
} from "@/hooks/use-plugins";
import { usePageTitle } from "@/hooks";
import {
  Plus,
  Package,
  Store,
  RefreshCw,
  Trash2,
  Download,
  AlertTriangle,
  ChevronDown,
  ChevronRight,
  Ellipsis,
} from "lucide-react";
import { registryDomainIcons } from "@/lib/registry-navigation";
import { pluralize } from "@/lib/formatting";
import type {
  Marketplace,
  MarketplaceCatalogEntry,
  InstalledPlugin,
  CreateMarketplaceRequest,
} from "@/lib/api/types";

// ============================================
// Helpers
// ============================================

const PluginsIcon = registryDomainIcons.plugins;

function truncateSha(sha: string | null | undefined): string {
  if (!sha) return "";
  return sha.slice(0, 8);
}

// ============================================
// Marketplace row card
// ============================================

function MarketplaceCard({
  marketplace,
  onRemove,
  onBrowse,
}: {
  marketplace: Marketplace;
  onRemove: (m: Marketplace) => void;
  onBrowse: (m: Marketplace) => void;
}) {
  const syncMutation = useSyncMarketplace();
  const updateMutation = useUpdateMarketplace(marketplace.id);
  const isDisabled = marketplace.status === "disabled";

  const handleToggle = () => {
    updateMutation.mutateAsync({ status: isDisabled ? "active" : "disabled" });
  };

  return (
    <Card>
      <CardHeader className="flex flex-row flex-wrap items-start justify-between gap-3 space-y-0">
        <div className="flex min-w-0 flex-1 items-center gap-3">
          <div className="flex h-9 w-9 shrink-0 items-center justify-center bg-primary/10">
            <Store className="h-5 w-5 text-primary" />
          </div>
          <div className="min-w-0">
            <CardTitle className="text-lg">
              <EntityIdentity
                value={marketplace.id}
                labelClassName={isDisabled ? "text-muted-foreground" : undefined}
              >
                {marketplace.name}
              </EntityIdentity>
            </CardTitle>
            <CardDescription className="text-sm truncate max-w-xs">
              {marketplaceSourceLabel(marketplace)}
            </CardDescription>
          </div>
        </div>
        <Badge variant={isDisabled ? "outline" : "default"}>{marketplace.status}</Badge>
      </CardHeader>
      <CardContent>
        <div className="space-y-1 text-sm text-muted-foreground mb-4">
          {marketplace.last_synced_at ? (
            <p>
              Last synced: {new Date(marketplace.last_synced_at).toLocaleString()}
              {marketplace.last_synced_sha && (
                <span className="ml-1 font-mono text-xs">
                  ({truncateSha(marketplace.last_synced_sha)})
                </span>
              )}
            </p>
          ) : (
            <p>Never synced</p>
          )}
        </div>
        <CardActions
          primary={
            <Button variant="outline" size="sm" onClick={() => onBrowse(marketplace)}>
              Browse
            </Button>
          }
          expanded={
            <>
              <Button
                variant="outline"
                size="sm"
                onClick={() => syncMutation.mutate(marketplace.id)}
                disabled={syncMutation.isPending}
              >
                <RefreshCw
                  className={`h-4 w-4 mr-1 ${syncMutation.isPending ? "animate-spin" : ""}`}
                />
                Sync now
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={handleToggle}
                disabled={updateMutation.isPending}
              >
                {isDisabled ? "Enable" : "Disable"}
              </Button>
              <Button variant="destructive" size="sm" onClick={() => onRemove(marketplace)}>
                <Trash2 className="h-4 w-4" />
              </Button>
            </>
          }
          collapsed={
            <DropdownMenu>
              <DropdownMenuTrigger
                render={<Button variant="outline" size="icon-sm" />}
                aria-label="More marketplace actions"
              >
                <Ellipsis />
              </DropdownMenuTrigger>
              <DropdownMenuPositioner align="end">
                <DropdownMenuContent>
                  <DropdownMenuItem
                    onClick={() => syncMutation.mutate(marketplace.id)}
                    disabled={syncMutation.isPending}
                  >
                    <RefreshCw className={syncMutation.isPending ? "animate-spin" : ""} />
                    {syncMutation.isPending ? "Syncing…" : "Sync now"}
                  </DropdownMenuItem>
                  <DropdownMenuItem onClick={handleToggle} disabled={updateMutation.isPending}>
                    {isDisabled ? "Enable" : "Disable"}
                  </DropdownMenuItem>
                  <DropdownMenuItem variant="destructive" onClick={() => onRemove(marketplace)}>
                    <Trash2 />
                    Remove
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenuPositioner>
            </DropdownMenu>
          }
        />
      </CardContent>
    </Card>
  );
}

function MarketplaceCardSkeleton() {
  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <div className="flex items-center gap-3">
          <Skeleton className="h-9 w-9" />
          <div className="space-y-2">
            <Skeleton className="h-5 w-32" />
            <Skeleton className="h-4 w-48" />
          </div>
        </div>
        <Skeleton className="h-5 w-16" />
      </CardHeader>
      <CardContent>
        <Skeleton className="h-4 w-48 mb-4" />
        <Skeleton className="h-8 w-24 ml-auto" />
      </CardContent>
    </Card>
  );
}

// ============================================
// Add Marketplace dialog
// ============================================

function AddMarketplaceDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [name, setName] = useState("");
  const [source, setSource] = useState("");
  const createMarketplace = useCreateMarketplace();

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const request: CreateMarketplaceRequest = {
      name: name.trim(),
      source_type: inferMarketplaceSourceType(source.trim()),
      source: source.trim(),
    };
    await createMarketplace.mutateAsync(request);
    onOpenChange(false);
    setName("");
    setSource("");
  };

  const handleOpenChange = (open: boolean) => {
    if (!open) {
      setName("");
      setSource("");
    }
    onOpenChange(open);
  };

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Add Marketplace</DialogTitle>
          <DialogDescription>
            Register a plugin marketplace by GitHub repo (<code>owner/repo</code>) or direct HTTPS
            URL to a <code>marketplace.json</code>.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="marketplace-name">Name</Label>
            <Input
              id="marketplace-name"
              value={name}
              onChange={(e: React.ChangeEvent<HTMLInputElement>) => setName(e.target.value)}
              placeholder="my-marketplace"
              required
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="marketplace-source">Source</Label>
            <Input
              id="marketplace-source"
              value={source}
              onChange={(e: React.ChangeEvent<HTMLInputElement>) => setSource(e.target.value)}
              placeholder="owner/repo or https://example.com/marketplace.json"
              required
            />
            <p className="text-xs text-muted-foreground">
              GitHub repo (e.g. <code>acme/plugins</code>) or a direct HTTPS URL.
            </p>
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => handleOpenChange(false)}>
              Cancel
            </Button>
            <Button
              type="submit"
              disabled={createMarketplace.isPending || !name.trim() || !source.trim()}
            >
              {createMarketplace.isPending ? "Adding..." : "Add Marketplace"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

// ============================================
// Remove Marketplace confirm dialog
// ============================================

function RemoveMarketplaceDialog({
  marketplace,
  open,
  onOpenChange,
}: {
  marketplace: Marketplace | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const deleteMarketplace = useDeleteMarketplace();

  const handleConfirm = async () => {
    if (!marketplace) return;
    await deleteMarketplace.mutateAsync(marketplace.id);
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Remove Marketplace</DialogTitle>
          <DialogDescription>
            Remove the marketplace <span className="font-medium">{marketplace?.name}</span>? The
            catalog will no longer be available, but already-installed plugins are not uninstalled.
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            variant="destructive"
            onClick={handleConfirm}
            disabled={deleteMarketplace.isPending}
          >
            {deleteMarketplace.isPending ? "Removing..." : "Remove"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// ============================================
// Marketplace catalog browser dialog
// ============================================

export function CatalogEntryRow({
  entry,
  marketplaceId,
  marketplaceName,
  installedPlugins,
}: {
  entry: MarketplaceCatalogEntry;
  marketplaceId: string;
  marketplaceName: string;
  installedPlugins: InstalledPlugin[];
}) {
  const installMutation = useInstallPlugin(marketplaceId, marketplaceName);
  const installStarted = useRef(false);
  const installedPlugin =
    installedPlugins.find((plugin) => plugin.name === entry.name) ??
    (installMutation.data?.plugin.name === entry.name ? installMutation.data.plugin : undefined);
  const isSameSource = installedPlugin?.marketplace === marketplaceName;
  const isUpgrade =
    isSameSource &&
    (installedPlugin.update_available ||
      (!!entry.version && !!installedPlugin.version && entry.version !== installedPlugin.version));
  const isSourceConflict = !!installedPlugin?.marketplace && !isSameSource;
  const isInstalled = entry.installed || (!!installedPlugin && !isUpgrade && !isSourceConflict);

  const handleInstall = () => {
    if (installStarted.current || isInstalled || isUpgrade || isSourceConflict) return;
    installStarted.current = true;
    installMutation.mutate(
      { marketplace_id: marketplaceId, plugin_name: entry.name },
      {
        onSettled: () => {
          installStarted.current = false;
        },
      },
    );
  };

  const installLabel = isUpgrade
    ? "Upgrade available"
    : isSourceConflict
      ? "Installed elsewhere"
      : isInstalled
        ? "Installed"
        : installMutation.isPending
          ? "Installing..."
          : "Install";

  return (
    <div className="py-3 border-b last:border-0">
      <div className="flex items-start justify-between gap-4">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 flex-wrap">
            <span className="font-medium text-sm">{entry.display_name ?? entry.name}</span>
            <span className="text-xs text-muted-foreground font-mono">{entry.name}</span>
            {entry.version && (
              <Badge variant="secondary" className="text-xs">
                v{entry.version}
              </Badge>
            )}
            {entry.category && (
              <Badge variant="outline" className="text-xs">
                {entry.category}
              </Badge>
            )}
            {(isInstalled || isUpgrade || isSourceConflict) && (
              <Badge variant="default" className="text-xs">
                {installLabel}
              </Badge>
            )}
          </div>
          {entry.description && (
            <p className="text-sm text-muted-foreground mt-1 line-clamp-2">{entry.description}</p>
          )}
          {entry.author && <p className="text-xs text-muted-foreground mt-1">by {entry.author}</p>}
        </div>
        <Button
          size="sm"
          variant={isInstalled || isUpgrade || isSourceConflict ? "outline" : "default"}
          disabled={isInstalled || isUpgrade || isSourceConflict || installMutation.isPending}
          onClick={handleInstall}
        >
          <Download className="h-4 w-4 mr-1" />
          {installLabel}
        </Button>
      </div>
      {isUpgrade && (
        <p className="mt-2 text-xs text-muted-foreground">
          Version {installedPlugin.version} is installed. Update it from Installed Plugins.
        </p>
      )}
      {isSourceConflict && (
        <p className="mt-2 text-xs text-muted-foreground">
          This plugin name is already installed from {installedPlugin.marketplace}.
        </p>
      )}
      {installMutation.isError && (
        <p role="alert" className="mt-2 text-sm text-destructive">
          Failed to install {entry.display_name ?? entry.name}: {installMutation.error.message}
        </p>
      )}
    </div>
  );
}

function MarketplaceCatalogDialog({
  marketplace,
  installedPlugins,
  open,
  onOpenChange,
}: {
  marketplace: Marketplace | null;
  installedPlugins: InstalledPlugin[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { data: entries, isLoading, error } = useMarketplaceCatalog(marketplace?.id ?? "");

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>{marketplace?.name} — Catalog</DialogTitle>
          <DialogDescription>
            Browse available plugins from this marketplace and install them.
          </DialogDescription>
        </DialogHeader>
        <div className="max-h-[60vh] overflow-y-auto">
          <QueryStateWrapper
            isLoading={isLoading}
            error={error}
            data={entries}
            errorMessagePrefix="Failed to load catalog"
            loadingSkeleton={
              <div className="space-y-4 py-2">
                {[...Array(4)].map((_, i) => (
                  <div key={i} className="flex items-start justify-between gap-4 py-3">
                    <div className="flex-1 space-y-2">
                      <Skeleton className="h-4 w-32" />
                      <Skeleton className="h-3 w-full" />
                    </div>
                    <Skeleton className="h-8 w-20 shrink-0" />
                  </div>
                ))}
              </div>
            }
            emptyState={
              <div className="text-center py-8">
                <Store className="mx-auto mb-3 h-10 w-10 text-muted-foreground" />
                <p className="text-muted-foreground">
                  {marketplace?.last_synced_at
                    ? "No plugins in this catalog"
                    : "Sync the marketplace to see available plugins"}
                </p>
              </div>
            }
          >
            {(items) => (
              <div>
                {items.map((entry) => (
                  <CatalogEntryRow
                    key={entry.name}
                    entry={entry}
                    marketplaceId={marketplace?.id ?? ""}
                    marketplaceName={marketplace?.name ?? ""}
                    installedPlugins={installedPlugins}
                  />
                ))}
              </div>
            )}
          </QueryStateWrapper>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Close
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// ============================================
// Installed plugin card
// ============================================

export function InstalledPluginCard({
  plugin,
  onUninstall,
}: {
  plugin: InstalledPlugin;
  onUninstall: (p: InstalledPlugin) => void;
}) {
  const patchMutation = usePatchInstalledPlugin(plugin.id);
  const updateMutation = useUpdateInstalledPlugin();
  const [warningsExpanded, setWarningsExpanded] = useState(false);
  const isEnabled = plugin.status === "active";

  const handleToggle = () => {
    patchMutation.mutate({ status: isEnabled ? "disabled" : "active" });
  };

  const handleUpdate = () => {
    updateMutation.mutate(plugin.id);
  };

  return (
    <Card>
      <CardHeader className="flex flex-row flex-wrap items-start justify-between gap-3 space-y-0">
        <div className="flex min-w-0 flex-1 items-center gap-3">
          <div className="flex h-9 w-9 shrink-0 items-center justify-center bg-primary/10">
            <Package className="h-5 w-5 text-primary" />
          </div>
          <div className="min-w-0">
            <CardTitle className="text-lg">
              <EntityIdentity
                value={plugin.capability_ref}
                labelClassName={!isEnabled ? "text-muted-foreground" : undefined}
              >
                {plugin.display_name ?? plugin.name}
              </EntityIdentity>
            </CardTitle>
            <CardDescription className="text-sm font-mono">{plugin.name}</CardDescription>
          </div>
        </div>
        <div className="flex max-w-full shrink-0 flex-wrap items-center justify-end gap-2">
          {plugin.warnings.length > 0 && (
            <Badge variant="outline" className="text-amber-600 border-amber-400 text-xs gap-1">
              <AlertTriangle className="h-3 w-3" />
              {plugin.warnings.length} warning{plugin.warnings.length !== 1 ? "s" : ""}
            </Badge>
          )}
          <Badge variant={isEnabled ? "default" : "outline"}>{plugin.status}</Badge>
        </div>
      </CardHeader>
      <CardContent>
        <div className="space-y-1 text-sm text-muted-foreground mb-3">
          {plugin.description && <p>{plugin.description}</p>}
          <div className="flex flex-wrap gap-x-4 gap-y-1 text-xs mt-2">
            {plugin.version && <span>Version: {plugin.version}</span>}
            {plugin.marketplace && <span>Marketplace: {plugin.marketplace}</span>}
            {plugin.pinned_sha && (
              <span>
                SHA: <span className="font-mono">{truncateSha(plugin.pinned_sha)}</span>
              </span>
            )}
          </div>
        </div>

        {plugin.warnings.length > 0 && (
          <div className="mb-3">
            <button
              type="button"
              onClick={() => setWarningsExpanded((v) => !v)}
              className="flex items-center gap-1 text-xs text-amber-600 hover:text-amber-700"
            >
              {warningsExpanded ? (
                <ChevronDown className="h-3 w-3" />
              ) : (
                <ChevronRight className="h-3 w-3" />
              )}
              Install warnings
            </button>
            {warningsExpanded && (
              <ul className="mt-1.5 space-y-1">
                {plugin.warnings.map((w, i) => (
                  <li key={i} className="text-xs text-amber-700 bg-amber-50 px-2 py-1 rounded">
                    {w}
                  </li>
                ))}
              </ul>
            )}
          </div>
        )}

        <div className="flex items-center justify-end gap-2">
          {plugin.update_available && (
            <Button
              variant="outline"
              size="sm"
              onClick={handleUpdate}
              disabled={updateMutation.isPending}
            >
              <RefreshCw
                className={`h-4 w-4 mr-1 ${updateMutation.isPending ? "animate-spin" : ""}`}
              />
              {updateMutation.isPending ? "Updating..." : "Update"}
            </Button>
          )}
          <Button
            variant="outline"
            size="sm"
            onClick={handleToggle}
            disabled={patchMutation.isPending}
          >
            {isEnabled ? "Disable" : "Enable"}
          </Button>
          <Button variant="destructive" size="sm" onClick={() => onUninstall(plugin)}>
            <Trash2 className="h-4 w-4 mr-1" />
            Uninstall
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}

function InstalledPluginCardSkeleton() {
  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <div className="flex items-center gap-3">
          <Skeleton className="h-9 w-9" />
          <div className="space-y-2">
            <Skeleton className="h-5 w-32" />
            <Skeleton className="h-4 w-24" />
          </div>
        </div>
        <Skeleton className="h-5 w-16" />
      </CardHeader>
      <CardContent>
        <Skeleton className="h-4 w-full mb-2" />
        <Skeleton className="h-4 w-48 mb-4" />
        <Skeleton className="h-8 w-24 ml-auto" />
      </CardContent>
    </Card>
  );
}

// ============================================
// Uninstall confirm dialog
// ============================================

function UninstallPluginDialog({
  plugin,
  open,
  onOpenChange,
}: {
  plugin: InstalledPlugin | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const deleteMutation = useDeleteInstalledPlugin();

  const handleConfirm = async () => {
    if (!plugin) return;
    await deleteMutation.mutateAsync(plugin.id);
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Uninstall Plugin</DialogTitle>
          <DialogDescription>
            Uninstall <span className="font-medium">{plugin?.display_name ?? plugin?.name}</span>?
            The capability <code>{plugin?.capability_ref}</code> will be removed. Agents referencing
            it will surface a dangling-ref error.
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button variant="destructive" onClick={handleConfirm} disabled={deleteMutation.isPending}>
            {deleteMutation.isPending ? "Uninstalling..." : "Uninstall"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// ============================================
// Page
// ============================================

export default function PluginsPage() {
  usePageTitle("Plugins");
  const [activeTab, setActiveTab] = useState<"installed" | "marketplaces">("installed");

  // Marketplace state
  const {
    data: marketplaces,
    isLoading: marketplacesLoading,
    error: marketplacesError,
  } = useMarketplaces();
  const [addMarketplaceOpen, setAddMarketplaceOpen] = useState(false);
  const [pendingRemoveMarketplace, setPendingRemoveMarketplace] = useState<Marketplace | null>(
    null,
  );
  const [browseMarketplace, setBrowseMarketplace] = useState<Marketplace | null>(null);

  // Installed plugins state
  const { data: plugins, isLoading: pluginsLoading, error: pluginsError } = useInstalledPlugins();
  const [pendingUninstallPlugin, setPendingUninstallPlugin] = useState<InstalledPlugin | null>(
    null,
  );

  const installedCount = plugins?.length ?? 0;
  const marketplaceCount = marketplaces?.length ?? 0;

  const sectionItems = [
    { value: "installed" as const, label: "Installed Plugins" },
    { value: "marketplaces" as const, label: "Marketplaces" },
  ];

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Plugins" }]} />

      <PageMasthead
        icon={<PluginsIcon />}
        title="Plugins"
        badges={
          <Badge variant="outline" className="font-mono">
            {installedCount}
          </Badge>
        }
        description="Install plugins from marketplaces to extend agents with skills, commands, and MCP servers packaged in the standard plugin format."
        meta={
          <>
            <span>{installedCount} installed</span>
            <span>{marketplaceCount} marketplaces</span>
          </>
        }
        actions={
          activeTab === "marketplaces" && (
            <Button variant="accent" onClick={() => setAddMarketplaceOpen(true)}>
              <Plus className="size-4" />
              Add Marketplace
            </Button>
          )
        }
      />

      <PageControlStrip>
        <SectionTabs
          value={activeTab}
          onValueChange={(v) => setActiveTab(v as "installed" | "marketplaces")}
          items={sectionItems}
        />
      </PageControlStrip>

      <PageMain>
        {/* ---- Installed Plugins tab ---- */}
        {activeTab === "installed" && (
          <QueryStateWrapper
            isLoading={pluginsLoading}
            error={pluginsError}
            data={plugins}
            errorMessagePrefix="Failed to load installed plugins"
            loadingSkeleton={
              <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
                {[...Array(3)].map((_, i) => (
                  <InstalledPluginCardSkeleton key={i} />
                ))}
              </div>
            }
            emptyState={
              <Card className="p-8 text-center">
                <Package className="mx-auto mb-4 h-12 w-12 text-muted-foreground" />
                <h2 className="mb-2 text-lg font-medium">No plugins installed</h2>
                <p className="mb-4 text-muted-foreground">
                  Browse a marketplace to install plugins into your organization.
                </p>
                <Button variant="outline" onClick={() => setActiveTab("marketplaces")}>
                  <Store className="mr-2 h-4 w-4" />
                  View Marketplaces
                </Button>
              </Card>
            }
          >
            {(items) => (
              <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
                {items.map((plugin) => (
                  <InstalledPluginCard
                    key={plugin.id}
                    plugin={plugin}
                    onUninstall={setPendingUninstallPlugin}
                  />
                ))}
              </div>
            )}
          </QueryStateWrapper>
        )}

        {/* ---- Marketplaces tab ---- */}
        {activeTab === "marketplaces" && (
          <QueryStateWrapper
            isLoading={marketplacesLoading}
            error={marketplacesError}
            data={marketplaces}
            errorMessagePrefix="Failed to load marketplaces"
            loadingSkeleton={
              <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
                {[...Array(3)].map((_, i) => (
                  <MarketplaceCardSkeleton key={i} />
                ))}
              </div>
            }
            emptyState={
              <Card className="p-8 text-center">
                <Store className="mx-auto mb-4 h-12 w-12 text-muted-foreground" />
                <h2 className="mb-2 text-lg font-medium">No marketplaces registered</h2>
                <p className="mb-4 text-muted-foreground">
                  Add a marketplace to browse and install plugins.
                </p>
                <Button onClick={() => setAddMarketplaceOpen(true)}>
                  <Plus className="mr-2 h-4 w-4" />
                  Add Marketplace
                </Button>
              </Card>
            }
          >
            {(items) => (
              <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
                {items.map((marketplace) => (
                  <MarketplaceCard
                    key={marketplace.id}
                    marketplace={marketplace}
                    onRemove={setPendingRemoveMarketplace}
                    onBrowse={setBrowseMarketplace}
                  />
                ))}
              </div>
            )}
          </QueryStateWrapper>
        )}
      </PageMain>

      <PageFooter>
        {activeTab === "installed" ? (
          <span>
            Showing {installedCount} {pluralize(installedCount, "plugin")}
          </span>
        ) : (
          <span>
            Showing {marketplaceCount} {pluralize(marketplaceCount, "marketplace")}
          </span>
        )}
      </PageFooter>

      {/* Marketplace dialogs */}
      <AddMarketplaceDialog open={addMarketplaceOpen} onOpenChange={setAddMarketplaceOpen} />
      <RemoveMarketplaceDialog
        marketplace={pendingRemoveMarketplace}
        open={pendingRemoveMarketplace !== null}
        onOpenChange={(open) => !open && setPendingRemoveMarketplace(null)}
      />
      <MarketplaceCatalogDialog
        marketplace={browseMarketplace}
        installedPlugins={plugins ?? []}
        open={browseMarketplace !== null}
        onOpenChange={(open) => !open && setBrowseMarketplace(null)}
      />

      {/* Installed plugin dialogs */}
      <UninstallPluginDialog
        plugin={pendingUninstallPlugin}
        open={pendingUninstallPlugin !== null}
        onOpenChange={(open) => !open && setPendingUninstallPlugin(null)}
      />
    </PageContainer>
  );
}
