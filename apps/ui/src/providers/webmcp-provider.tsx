"use client";

import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { usePathname, useRouter } from "next/navigation";
import { searchAgents, getAgent } from "@/lib/api/agents";
import { getSession, listSessions } from "@/lib/api/sessions";
import { useAuth } from "@/providers/auth-provider";
import { useOrg } from "@/providers/org-provider";
import { useFeatureFlag } from "@/providers/feature-flags-provider";
import { useWebMcpTool } from "@/hooks/use-webmcp-tool";
import type { WebMcpToolDefinition } from "@/lib/webmcp/types";
import {
  WebMcpContext,
  useWebMcp,
  type WebMcpApprovalRequest,
  type WebMcpContextValue,
} from "./webmcp-context";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";

const PAGE_ROUTES = {
  agents: "/agents",
  sessions: "/sessions",
  chat: "/chats",
} as const;

type PageName = keyof typeof PAGE_ROUTES;

interface PendingApproval {
  request: WebMcpApprovalRequest;
  resolve: () => void;
  reject: (error: Error) => void;
}

function abortError(message: string): DOMException {
  return new DOMException(message, "AbortError");
}

function requiredString(input: Record<string, unknown>, key: string, maxLength: number): string {
  const value = input[key];
  if (typeof value !== "string" || !value.trim()) {
    throw new TypeError(`${key} must be a non-empty string`);
  }
  return value.trim().slice(0, maxLength);
}

function boundedLimit(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return 6;
  return Math.max(1, Math.min(10, Math.floor(value)));
}

function shortText(value: string | null | undefined, maxLength = 120): string {
  if (!value) return "Untitled";
  return value.length <= maxLength ? value : `${value.slice(0, maxLength - 1)}…`;
}

export function getEverrunsPageContext(pathname: string): {
  page: string;
  resource_type?: "agent" | "session";
  resource_id?: string;
} {
  const parts = pathname.split("/").filter(Boolean);
  if (parts[0] === "agents" && parts[1]) {
    return { page: "agent", resource_type: "agent", resource_id: parts[1] };
  }
  if (parts[0] === "sessions" && parts[1]) {
    return { page: "session", resource_type: "session", resource_id: parts[1] };
  }
  return { page: parts[0] ?? "chats" };
}

function WebMcpShellTools({ enabled }: { enabled: boolean }) {
  const router = useRouter();
  const pathname = usePathname();
  const { currentOrg } = useOrg();
  const webmcp = useWebMcp();
  const boundOrgId = currentOrg?.public_id;
  const scopeKey = `${boundOrgId ?? "none"}:${pathname}`;

  const contextTool = useMemo<WebMcpToolDefinition>(
    () => ({
      name: "everruns_get_context",
      description:
        "Return the active Everruns page and organization. Use before page-specific actions.",
      inputSchema: { type: "object", properties: {}, additionalProperties: false },
      annotations: {
        readOnlyHint: true,
        idempotentHint: true,
        untrustedContentHint: true,
      },
      execute: async () => {
        webmcp.assertBinding(webmcp.bindingToken);
        if (!boundOrgId) throw abortError("The active organization changed");
        return {
          organization_id: boundOrgId,
          organization_name: shortText(currentOrg?.name),
          ...getEverrunsPageContext(pathname),
        };
      },
    }),
    [boundOrgId, currentOrg?.name, pathname, webmcp],
  );

  const searchTool = useMemo<WebMcpToolDefinition>(
    () => ({
      name: "everruns_search",
      description: "Search accessible Everruns agents and sessions by name or title.",
      inputSchema: {
        type: "object",
        properties: {
          query: { type: "string", description: "Name or title text to find." },
          resource_type: { type: "string", enum: ["agent", "session", "all"] },
          limit: { type: "number", minimum: 1, maximum: 10 },
        },
        required: ["query"],
        additionalProperties: false,
      },
      annotations: {
        readOnlyHint: true,
        idempotentHint: true,
        untrustedContentHint: true,
      },
      execute: async (input) => {
        // THREAT[TM-WEB-015]: return bounded identity metadata only; prompts, transcripts,
        // descriptions, and connection configuration never enter browser-agent context.
        webmcp.assertBinding(webmcp.bindingToken);
        if (!boundOrgId) throw abortError("The active organization changed");
        const query = requiredString(input, "query", 100);
        const requestedType = input.resource_type;
        const resourceType =
          requestedType === "agent" || requestedType === "session" ? requestedType : "all";
        const limit = boundedLimit(input.limit);
        const perTypeLimit = resourceType === "all" ? Math.ceil(limit / 2) : limit;
        const [agents, sessions] = await Promise.all([
          resourceType !== "session" ? searchAgents(query, perTypeLimit) : null,
          resourceType !== "agent" ? listSessions({ search: query, limit: perTypeLimit }) : null,
        ]);
        const results = [
          ...(agents?.data ?? []).map((agent) => ({
            resource_type: "agent" as const,
            resource_id: agent.id,
            name: shortText(agent.display_name ?? agent.name),
            status: agent.status,
          })),
          ...(sessions?.data ?? []).map((session) => ({
            resource_type: "session" as const,
            resource_id: session.id,
            name: shortText(session.title),
            status: session.status,
          })),
        ].slice(0, limit);
        webmcp.assertBinding(webmcp.bindingToken);
        return { query, results, truncated: results.length === limit };
      },
    }),
    [boundOrgId, webmcp],
  );

  const openTool = useMemo<WebMcpToolDefinition>(
    () => ({
      name: "everruns_open",
      description: "Open a known Everruns page, agent, or session in the visible UI.",
      inputSchema: {
        type: "object",
        properties: {
          target_type: {
            type: "string",
            enum: ["page", "agent", "session"],
          },
          page: { type: "string", enum: Object.keys(PAGE_ROUTES) },
          resource_id: { type: "string" },
        },
        required: ["target_type"],
        additionalProperties: false,
      },
      annotations: { readOnlyHint: true, idempotentHint: true },
      execute: async (input) => {
        webmcp.assertBinding(webmcp.bindingToken);
        if (!boundOrgId) throw abortError("The active organization changed");
        let href: string;
        if (input.target_type === "page") {
          const page = input.page;
          if (typeof page !== "string" || !(page in PAGE_ROUTES)) {
            throw new TypeError("page must be a supported Everruns page");
          }
          href = PAGE_ROUTES[page as PageName];
        } else if (input.target_type === "agent") {
          const resourceId = requiredString(input, "resource_id", 64);
          if (!/^agent_[0-9a-f]{32}$/.test(resourceId)) {
            throw new TypeError("resource_id must be an agent ID");
          }
          await getAgent(resourceId);
          href = `/agents/${resourceId}`;
        } else if (input.target_type === "session") {
          const resourceId = requiredString(input, "resource_id", 64);
          if (!/^session_[0-9a-f]{32}$/.test(resourceId)) {
            throw new TypeError("resource_id must be a session ID");
          }
          await getSession(resourceId);
          href = `/sessions/${resourceId}/transcript`;
        } else {
          throw new TypeError("target_type must be page, agent, or session");
        }
        webmcp.assertBinding(webmcp.bindingToken);
        router.push(href);
        return { opened: true, path: href };
      },
    }),
    [boundOrgId, router, webmcp],
  );

  useWebMcpTool(contextTool, { enabled, scopeKey });
  useWebMcpTool(searchTool, { enabled, scopeKey: boundOrgId });
  useWebMcpTool(openTool, { enabled, scopeKey: boundOrgId });
  return null;
}

export function WebMcpProvider({ children }: { children: ReactNode }) {
  const pathname = usePathname();
  const { isAuthenticated, user } = useAuth();
  const { currentOrg } = useOrg();
  const featureEnabled = useFeatureFlag("webmcp");
  const enabled = featureEnabled && isAuthenticated && !!user && !!currentOrg;
  const [pending, setPending] = useState<PendingApproval | null>(null);
  const pendingRef = useRef<PendingApproval | null>(null);
  // THREAT[TM-WEB-016]: callbacks capture this token and compare it again immediately before
  // acting, so route, user, or organization changes invalidate stale browser-agent work.
  const bindingKey = `${user?.id ?? "none"}:${currentOrg?.public_id ?? "none"}:${pathname}`;
  const bindingRef = useRef(bindingKey);
  bindingRef.current = bindingKey;

  const assertBinding = useCallback((token: string) => {
    if (bindingRef.current !== token) {
      throw abortError("The WebMCP page context changed");
    }
  }, []);

  const settle = useCallback((approved: boolean) => {
    const current = pendingRef.current;
    if (!current) return;
    pendingRef.current = null;
    setPending(null);
    if (approved) current.resolve();
    else current.reject(abortError("The user rejected the WebMCP action"));
  }, []);

  const requestApproval = useCallback((request: WebMcpApprovalRequest) => {
    // THREAT[TM-WEB-013]: side-effecting tools cannot act solely on browser-agent authority;
    // the signed-in user must approve the visible action and target in Everruns.
    if (pendingRef.current) {
      return Promise.reject(new Error("Another WebMCP action is awaiting approval"));
    }
    return new Promise<void>((resolve, reject) => {
      const next = { request, resolve, reject };
      pendingRef.current = next;
      setPending(next);
    });
  }, []);

  useEffect(() => {
    return () => {
      const current = pendingRef.current;
      if (!current) return;
      pendingRef.current = null;
      setPending(null);
      current.reject(abortError("The WebMCP page context changed"));
    };
  }, [bindingKey]);

  const value = useMemo<WebMcpContextValue>(
    () => ({ enabled, bindingToken: bindingKey, assertBinding, requestApproval }),
    [assertBinding, bindingKey, enabled, requestApproval],
  );

  return (
    <WebMcpContext.Provider value={value}>
      <WebMcpShellTools enabled={enabled} />
      {children}
      <Dialog open={!!pending} onOpenChange={(open) => !open && settle(false)}>
        <DialogContent showCloseButton={false} data-testid="webmcp-approval-dialog">
          <DialogHeader>
            <DialogTitle>{pending?.request.title}</DialogTitle>
            <DialogDescription>{pending?.request.description}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => settle(false)}>
              Reject
            </Button>
            <Button
              variant={pending?.request.destructive ? "destructive" : "default"}
              onClick={() => settle(true)}
            >
              {pending?.request.confirmLabel ?? "Approve"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </WebMcpContext.Provider>
  );
}
