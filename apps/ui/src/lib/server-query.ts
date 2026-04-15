import type { QueryClient } from "@tanstack/react-query";
import { cookies, headers } from "next/headers";
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
  apiBaseUrl: string;
  cookieHeader: string;
  orgCookieId: string | null;
}

export function createServerQueryClient() {
  return createAppQueryClient();
}

export function getRequestOrigin(headersList: Headers): string {
  const host = headersList.get("x-forwarded-host") ?? headersList.get("host") ?? "localhost";
  const proto = headersList.get("x-forwarded-proto") ?? "http";
  return `${proto}://${host}`;
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
  const [headersList, cookieStore] = await Promise.all([headers(), cookies()]);

  return {
    apiBaseUrl: `${getRequestOrigin(headersList)}/api`,
    cookieHeader: cookieStore.toString(),
    orgCookieId: cookieStore.get("everruns_org")?.value ?? null,
  };
}

async function serverRequestJson<T>(context: ServerRequestContext, endpoint: string): Promise<T> {
  const response = await fetch(`${context.apiBaseUrl}${endpoint}`, {
    cache: "no-store",
    headers: context.cookieHeader
      ? {
          cookie: context.cookieHeader,
        }
      : undefined,
  });

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
