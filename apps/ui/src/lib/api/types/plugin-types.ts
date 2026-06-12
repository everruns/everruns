// Plugin Marketplace and Installed Plugin types
// See specs/plugins.md for the data model and API sketch.

// ============================================
// Marketplace types
// ============================================

/** Status of a registered marketplace */
export type MarketplaceStatus = "active" | "disabled";

/** Where a marketplace's catalog is fetched from. `local_path` is dev-only. */
export type MarketplaceSourceType = "github" | "url" | "local_path";

/** Source configuration; the populated field follows `source_type` */
export interface MarketplaceSource {
  /** GitHub repo `owner/repo` (source_type "github") */
  repo?: string;
  /** Direct HTTPS URL to a marketplace.json (source_type "url") */
  url?: string;
  /** Local filesystem path (source_type "local_path", dev-only) */
  path?: string;
}

/** A registered plugin marketplace (org-scoped catalog) */
export interface Marketplace {
  id: string;
  name: string;
  source_type: MarketplaceSourceType;
  source: MarketplaceSource;
  status: MarketplaceStatus;
  /** ISO timestamp of the last successful sync */
  last_synced_at: string | null;
  /** Resolved commit SHA if source is a git repo */
  last_synced_sha: string | null;
  created_at: string;
  updated_at: string;
}

/** Request to register a new marketplace */
export interface CreateMarketplaceRequest {
  name: string;
  source_type: MarketplaceSourceType;
  /** `owner/repo` for github, HTTPS URL for url, filesystem path for local_path */
  source: string;
}

/** Request to update a marketplace */
export interface UpdateMarketplaceRequest {
  name?: string;
  status?: MarketplaceStatus;
}

// ============================================
// Marketplace catalog entry types
// ============================================

/** A plugin entry as it appears in a marketplace's synced catalog */
export interface MarketplaceCatalogEntry {
  /** Plugin name (kebab-case) */
  name: string;
  /** Human-readable display name */
  display_name: string | null;
  description: string | null;
  version: string | null;
  author: string | null;
  category: string | null;
  /** Whether this plugin is already installed in the current org */
  installed: boolean;
}

// ============================================
// Installed plugin types
// ============================================

/** Status of an installed plugin */
export type InstalledPluginStatus = "active" | "disabled";

/** Install warning about unsupported plugin components (e.g. hooks, lspServers) */
export type InstalledPluginWarning = string;

/** An installed plugin (compiled into the capability registry) */
export interface InstalledPlugin {
  id: string;
  /** Kebab-case plugin name; capability ref is `plugin:{name}` */
  name: string;
  display_name: string | null;
  description: string | null;
  version: string | null;
  /** Pinned commit SHA at install time */
  pinned_sha: string | null;
  /** Name of the marketplace this was installed from */
  marketplace: string | null;
  /** Capability reference: `plugin:{name}` */
  capability_ref: string;
  status: InstalledPluginStatus;
  /** Install-time warnings for unsupported plugin components */
  warnings: InstalledPluginWarning[];
  /** True when the marketplace catalog has a newer version or SHA */
  update_available: boolean;
  created_at: string;
  updated_at: string;
}

/** Request to install a plugin from a marketplace catalog entry */
export interface InstallPluginRequest {
  marketplace_id: string;
  plugin_name: string;
}

/** Request to update an installed plugin's metadata */
export interface UpdateInstalledPluginRequest {
  status?: InstalledPluginStatus;
}
