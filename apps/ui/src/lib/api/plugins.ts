// Plugin Marketplaces and Installed Plugins API
// All endpoints follow specs/plugins.md § "API sketch".
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org).
//
// Centralising endpoint paths and request/response types here means a rename
// of any URL or field is a single-file change.

import { api } from "./client";
import { createCrudApi } from "./crud";
import type {
  Marketplace,
  MarketplaceSourceType,
  CreateMarketplaceRequest,
  UpdateMarketplaceRequest,
  MarketplaceCatalogEntry,
  InstalledPlugin,
  InstallPluginRequest,
  UpdateInstalledPluginRequest,
  ListResponse,
} from "./types";

// ============================================
// Endpoint constants
// ============================================

export const PLUGIN_MARKETPLACES_BASE = "/v1/plugin_marketplaces";
export const PLUGINS_BASE = "/v1/plugins";

// ============================================
// Marketplace CRUD (list/create/patch/delete)
// ============================================

export const marketplacesCrudApi = createCrudApi<
  Marketplace,
  CreateMarketplaceRequest,
  UpdateMarketplaceRequest
>(PLUGIN_MARKETPLACES_BASE);

export const getMarketplaces = marketplacesCrudApi.list;
export const getMarketplace = marketplacesCrudApi.get;
export const createMarketplace = marketplacesCrudApi.create;
export const updateMarketplace = marketplacesCrudApi.update;
export const deleteMarketplace = marketplacesCrudApi.delete;

// ============================================
// Marketplace sync
// ============================================

/** POST /v1/plugin_marketplaces/{id}/sync — trigger explicit re-sync */
export async function syncMarketplace(id: string): Promise<Marketplace> {
  const response = await api.post<Marketplace>(`${PLUGIN_MARKETPLACES_BASE}/${id}/sync`);
  return response.data;
}

// ============================================
// Marketplace catalog
// ============================================

/** GET /v1/plugin_marketplaces/{id}/plugins — list synced catalog entries */
export async function getMarketplaceCatalog(
  marketplaceId: string,
): Promise<MarketplaceCatalogEntry[]> {
  const response = await api.get<ListResponse<MarketplaceCatalogEntry>>(
    `${PLUGIN_MARKETPLACES_BASE}/${marketplaceId}/plugins`,
  );
  return response.data.data;
}

// ============================================
// Installed plugins
// ============================================

export const installedPluginsCrudApi = createCrudApi<
  InstalledPlugin,
  InstallPluginRequest,
  UpdateInstalledPluginRequest
>(PLUGINS_BASE);

export const getInstalledPlugins = installedPluginsCrudApi.list;
export const getInstalledPlugin = installedPluginsCrudApi.get;
export const deleteInstalledPlugin = installedPluginsCrudApi.delete;

/** POST /v1/plugins — install a plugin from a marketplace catalog entry */
export async function installPlugin(request: InstallPluginRequest): Promise<InstalledPlugin> {
  const response = await api.post<InstalledPlugin>(PLUGINS_BASE, request);
  return response.data;
}

/** PATCH /v1/plugins/{id} — enable/disable an installed plugin */
export async function patchInstalledPlugin(
  id: string,
  request: UpdateInstalledPluginRequest,
): Promise<InstalledPlugin> {
  const response = await api.patch<InstalledPlugin>(`${PLUGINS_BASE}/${id}`, request);
  return response.data;
}

/** POST /v1/plugins/{id}/update — re-install at newest catalog version */
export async function updateInstalledPlugin(id: string): Promise<InstalledPlugin> {
  const response = await api.post<InstalledPlugin>(`${PLUGINS_BASE}/${id}/update`);
  return response.data;
}

/** Human-readable label for a marketplace's source */
export function marketplaceSourceLabel(marketplace: Marketplace): string {
  const { repo, url, path } = marketplace.source;
  return repo ?? url ?? path ?? "";
}

/** Infer the source type from free-text source input: `owner/repo` is GitHub, otherwise a URL */
export function inferMarketplaceSourceType(source: string): MarketplaceSourceType {
  if (/^[\w.-]+\/[\w.-]+$/.test(source)) {
    return "github";
  }
  if (source.startsWith("/") || source.startsWith("./")) {
    return "local_path";
  }
  return "url";
}
