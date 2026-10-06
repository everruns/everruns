import type { QueryClient } from "@tanstack/react-query";
import { cookies } from "next/headers";
import { authQueryKeys } from "@/lib/auth-query-keys";
import { createAppQueryClient } from "@/lib/query-client";
import type {
  AuthConfigResponse,
  ListResponse,
  OrganizationMembership,
  PaginatedResponse,
  UserInfoResponse,
} from "@/lib/api/types";

const DEFAULT_ORG_PUBLIC_ID = "org_00000000000000000000000000000001";

export interface ServerRequestContext {
  /**
   * Deployment-trusted API base (`.../api`), or `null` when none is configured.
   * `null` disables server prefetch; the client fetches after hydration.
   */
  apiBaseUrl: string | null;
  cookieHeader: string;
  orgCookieId: string | null;
}

/**
 * A query client for a server render, hashing its entries under the org the
 * request arrived for (the `everruns_org` cookie).
 *
 * It must be the same org the browser publishes on its first render — which is
 * this same cookie value, handed to `OrgProvider` as `initialOrgId` — or the
 * dehydrated entries hash differently on hydration and every seeded page
 * refetches what the server already fetched.
 */
export function createServerQueryClient(context: ServerRequestContext) {
  return createAppQueryClient(() => context.orgCookieId);
}

type TrustedApiEnv = Partial<Record<"UI_SERVER_API_URL" | "PUBLIC_APP_URL", string>>;

function parseTrustedUrl(value: string | undefined): URL | null {
  const trimmed = value?.trim();
  if (!trimmed) return null;
  let url: URL;
  try {
    url = new URL(trimmed);
  } catch {
    return null;
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") return null;
  if (url.username || url.password || url.search || url.hash) return null;
  return url;
}

/**
 * The API base that server renders call, taken only from deployment configuration.
 *
 * THREAT[TM-WEB-019]: server renders forward the browser's auth cookies, so the
 * destination must never come from the request. The Host header is
 * attacker-selectable (catch-all ingress routes forward any Host to the UI), and
 * deriving the origin from it let a crafted request aim cookie-bearing fetches
 * at an arbitrary service. Resolution order:
 *   1. `UI_SERVER_API_URL`: full API base, may be internal
 *      (e.g. `http://server:9000/api`).
 *   2. `PUBLIC_APP_URL` + `/api`: the public app origin the server already uses,
 *      exported to the UI by the local dev stack.
 * Neither set (or invalid) returns `null`: fail closed and skip server prefetch,
 * never fall back to Host.
 */
export function resolveTrustedApiBaseUrl(env: TrustedApiEnv): string | null {
  const explicit = parseTrustedUrl(env.UI_SERVER_API_URL);
  if (explicit) {
    return `${explicit.origin}${explicit.pathname}`.replace(/\/+$/, "");
  }
  const publicApp = parseTrustedUrl(env.PUBLIC_APP_URL);
  if (publicApp && publicApp.pathname === "/") {
    return `${publicApp.origin}/api`;
  }
  return null;
}

function runtimeTrustedApiEnv(): TrustedApiEnv {
  // Read through the environment object so standalone images pick the value up
  // at runtime instead of baking in a build-time value (same as proxy.ts).
  const runtimeEnv = process.env;
  return {
    UI_SERVER_API_URL: runtimeEnv.UI_SERVER_API_URL,
    PUBLIC_APP_URL: runtimeEnv.PUBLIC_APP_URL,
  };
}

export function resolveCurrentOrgId(
  organizations: OrganizationMembership[],
  preferredOrgId?: string | null,
): string | null {
  if (preferredOrgId && organizations.some((org) => org.public_id === preferredOrgId)) {
    return preferredOrgId;
  }

  const defaultOrg = organizations.find((org) => org.public_id === DEFAULT_ORG_PUBLIC_ID);
  return defaultOrg?.public_id ?? organizations[0]?.public_id ?? null;
}

export async function getServerRequestContext(): Promise<ServerRequestContext> {
  // Request headers (Host, X-Forwarded-*) are deliberately not consulted.
  const cookieStore = await cookies();

  return {
    apiBaseUrl: resolveTrustedApiBaseUrl(runtimeTrustedApiEnv()),
    cookieHeader: cookieStore.toString(),
    orgCookieId: cookieStore.get("everruns_org")?.value ?? null,
  };
}

async function serverRequestJson<T>(context: ServerRequestContext, endpoint: string): Promise<T> {
  if (!context.apiBaseUrl) {
    throw new Error("Server prefetch disabled: no trusted API base URL configured");
  }

  const response = await fetch(`${context.apiBaseUrl}${endpoint}`, {
    cache: "no-store",
    // A followed redirect could carry the forwarded cookie to another origin or
    // steer the request at an internal target. Never follow; treat as failure.
    redirect: "manual",
    headers: context.cookieHeader
      ? {
          cookie: context.cookieHeader,
        }
      : undefined,
  });

  if (response.type === "opaqueredirect" || (response.status >= 300 && response.status < 400)) {
    throw new Error(`Server request for ${endpoint} was redirected; refusing to follow`);
  }

  if (!response.ok) {
    throw new Error(
      `Server request failed for ${endpoint}: ${response.status} ${response.statusText}`,
    );
  }

  return response.json();
}

export async function serverGet<T>(context: ServerRequestContext, endpoint: string): Promise<T> {
  return serverRequestJson<T>(context, endpoint);
}

export async function serverGetList<T>(
  context: ServerRequestContext,
  endpoint: string,
): Promise<T[]> {
  const response = await serverRequestJson<ListResponse<T>>(context, endpoint);
  return response.data;
}

export async function serverGetPaginated<T>(
  context: ServerRequestContext,
  endpoint: string,
): Promise<PaginatedResponse<T>> {
  return serverRequestJson<PaginatedResponse<T>>(context, endpoint);
}

export async function seedQueryData<T>(
  queryClient: QueryClient,
  queryKey: readonly unknown[],
  load: () => Promise<T>,
): Promise<T | undefined> {
  try {
    const data = await load();
    queryClient.setQueryData(queryKey, data);
    return data;
  } catch {
    return undefined;
  }
}

export async function prefetchAuthBootstrap(
  queryClient: QueryClient,
  context: ServerRequestContext,
): Promise<{ config?: AuthConfigResponse; user?: UserInfoResponse; currentOrgId: string | null }> {
  const config = await seedQueryData(queryClient, authQueryKeys.config(), () =>
    serverGet<AuthConfigResponse>(context, "/v1/auth/config"),
  );
  const user = await seedQueryData(queryClient, authQueryKeys.user(), () =>
    serverGet<UserInfoResponse>(context, "/v1/auth/me"),
  );

  return {
    config,
    user,
    currentOrgId: resolveCurrentOrgId(user?.organizations ?? [], context.orgCookieId),
  };
}
