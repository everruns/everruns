// Plugin Marketplaces and Installed Plugins API
// All endpoints follow knowledge/integrations/plugins.md § "API sketch".
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/platform/users/me/switch-org).
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
  PluginEnvironment,
  PluginOption,
  UpdateInstalledPluginRequest,
  ListResponse,
} from "./types";

// ============================================
// Endpoint constants
// ============================================

export const PLUGIN_MARKETPLACES_BASE = "/v1/platform/plugin_marketplaces";
export const PLUGINS_BASE = "/v1/platform/plugins";

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

/** POST /v1/platform/plugin_marketplaces/{id}/sync — trigger explicit re-sync */
export async function syncMarketplace(id: string): Promise<Marketplace> {
  const response = await api.post<Marketplace>(`${PLUGIN_MARKETPLACES_BASE}/${id}/sync`);
  return response.data;
}

// ============================================
// Marketplace catalog
// ============================================

/** GET /v1/platform/plugin_marketplaces/{id}/plugins — list synced catalog entries */
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

/** POST /v1/platform/plugins — install a plugin from a marketplace catalog entry */
export async function installPlugin(request: InstallPluginRequest): Promise<InstalledPlugin> {
  const response = await api.post<InstalledPlugin>(PLUGINS_BASE, request);
  return response.data;
}

/** PATCH /v1/platform/plugins/{id} — enable/disable an installed plugin */
export async function patchInstalledPlugin(
  id: string,
  request: UpdateInstalledPluginRequest,
): Promise<InstalledPlugin> {
  const response = await api.patch<InstalledPlugin>(`${PLUGINS_BASE}/${id}`, request);
  return response.data;
}

/** POST /v1/platform/plugins/{id}/update — re-install at newest catalog version */
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

// ============================================
// A person's own plugins (Settings > Personal > Plugins)
// ============================================

export const MY_PLUGINS_BASE = "/v1/platform/users/me/plugins";

/** GET /v1/platform/users/me/plugins — approved plugins with your own choices */
export async function getMyPlugins(): Promise<PluginOption[]> {
  const response = await api.get<PluginOption[]>(MY_PLUGINS_BASE);
  return response.data;
}

/** PUT /v1/platform/users/me/plugins/{id} — add a plugin for yourself, or take it out */
export async function setMyPlugin(id: string, enabled: boolean): Promise<PluginOption[]> {
  const response = await api.put<PluginOption[]>(`${MY_PLUGINS_BASE}/${id}`, { enabled });
  return response.data;
}

/** The harness level that provides an environment, as people see it. */
export function pluginEnvironmentLabel(environment: PluginEnvironment | undefined): string {
  switch (environment) {
    case "conversation":
      return "Conversation";
    case "workspace":
      return "Worker";
    case "virtual-shell":
      return "Bashkit Worker";
    default:
      return "Sandbox Worker";
  }
}

/** The ref an agent stores to take every plugin the organization approved. */
export const ALL_APPROVED_PLUGINS_REF = "plugin:_all";

const ENVIRONMENT_ORDER: PluginEnvironment[] = [
  "conversation",
  "workspace",
  "virtual-shell",
  "sandbox",
];

/**
 * What an agent with these capability refs provides, mirroring the server's
 * `PluginEnvironment::provided_by`. Advisory in the UI: the server decides.
 */
export function providedEnvironment(refs: readonly string[]): PluginEnvironment {
  let level = 0;
  for (const ref of refs) {
    const next =
      ref === "session_sandbox"
        ? 3
        : ref === "bashkit_shell" || ref === "virtual_bash"
          ? 2
          : ref === "session_file_system"
            ? 1
            : 0;
    level = Math.max(level, next);
  }
  return ENVIRONMENT_ORDER[level];
}

/** Whether an agent providing `provided` can run a plugin that needs `needed`. */
export function environmentSatisfied(
  needed: PluginEnvironment | undefined,
  provided: PluginEnvironment,
): boolean {
  return ENVIRONMENT_ORDER.indexOf(provided) >= ENVIRONMENT_ORDER.indexOf(needed ?? "sandbox");
}
