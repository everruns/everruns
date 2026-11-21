"use client";

/**
 * An MCP App view under the tool call that produced it (knowledge/integrations/
 * mcp-apps-host.md).
 *
 * Decisions:
 * - The view's HTML is loaded from the server when the row renders, not
 *   stored with the event; until the view says it is ready a skeleton holds
 *   its place, and if it fails the text result shows with a Retry.
 * - The source server is always named above the frame, so a view cannot pass
 *   itself off as part of the console.
 * - Read-only and app-only tools a view calls run directly; any other tool
 *   asks the person first. A view's `ui/message` only fills the composer.
 * - Only the session's owner lets a view call tools; everyone else sees the
 *   view with the data the call already returned.
 * - Reads its session from a small context the session provider fills; rows
 *   outside a session (dev previews, Work Chat) show no view.
 */

import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { AppWindow, Maximize2, RotateCw } from "lucide-react";
import type { ToolCompletedData } from "@/lib/api/types";
import {
  callMcpAppTool,
  callToolResultFor,
  COMPOSE_DRAFT_EVENT,
  getMcpAppView,
  readMcpAppResource,
  type ComposeDraftDetail,
  type McpAppCall,
} from "@/lib/mcp-apps";
import {
  McpAppFrame,
  type McpAppDisplayMode,
  type McpAppHostHandlers,
} from "@/components/mcp/mcp-app-frame";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";
import { useTheme } from "@/providers/theme-provider";
import { cn } from "@/lib/utils";

/** How long a loaded view gets to say it is ready before the text result shows. */
const VIEW_START_TIMEOUT_MS = 10_000;

/** The session whose MCP servers views load from. */
export const McpAppSessionContext = createContext<string | undefined>(undefined);

interface PendingConfirm {
  title: string;
  description?: string;
  resolve: (confirmed: boolean) => void;
}

export function McpAppView({
  app,
  toolCallId,
  toolInput,
  toolResult,
}: {
  app: McpAppCall;
  toolCallId: string;
  toolInput: Record<string, unknown>;
  toolResult: ToolCompletedData;
}) {
  const sessionId = useContext(McpAppSessionContext);
  if (!sessionId) return null;
  return (
    <McpAppViewInSession
      sessionId={sessionId}
      app={app}
      toolCallId={toolCallId}
      toolInput={toolInput}
      toolResult={toolResult}
    />
  );
}

function serverLabel(server: string): string {
  return server.charAt(0).toUpperCase() + server.slice(1).replace(/_/g, " ");
}

function McpAppViewInSession({
  sessionId,
  app,
  toolCallId,
  toolInput,
  toolResult,
}: {
  sessionId: string;
  app: McpAppCall;
  toolCallId: string;
  toolInput: Record<string, unknown>;
  toolResult: ToolCompletedData;
}) {
  const ui = useUiTranslations();
  const { locale } = useLocale();
  const { theme } = useTheme();
  const [ready, setReady] = useState(false);
  const [height, setHeight] = useState(360);
  const [mode, setMode] = useState<McpAppDisplayMode>("inline");
  const [showData, setShowData] = useState(false);
  const [confirm, setConfirm] = useState<PendingConfirm | null>(null);

  const view = useQuery({
    queryKey: ["mcp-app-view", sessionId, app.server, app.resource_uri],
    queryFn: () => getMcpAppView(sessionId, app),
    staleTime: 5 * 60_000,
    retry: false,
  });

  // A view whose script never runs (blocked by a policy, broken bundle) would
  // leave the skeleton forever; after a while show the text result instead.
  // Keyed by the load it belongs to, so a retry starts with a fresh clock.
  const [stalledLoad, setStalledLoad] = useState<number | null>(null);
  const stalled = stalledLoad !== null && stalledLoad === view.dataUpdatedAt;
  useEffect(() => {
    if (!view.data || ready) return;
    const load = view.dataUpdatedAt;
    const timer = window.setTimeout(() => setStalledLoad(load), VIEW_START_TIMEOUT_MS);
    return () => window.clearTimeout(timer);
  }, [view.data, view.dataUpdatedAt, ready]);

  const callResult = useMemo(
    () => callToolResultFor(toolResult.result, toolResult.error, app),
    [toolResult.result, toolResult.error, app],
  );
  const label = serverLabel(app.server);
  const canCallTools = view.data?.can_call_tools ?? false;

  const askToRun = useCallback(
    (title: string, description?: string) =>
      new Promise<boolean>((resolve) => setConfirm({ title, description, resolve })),
    [],
  );

  const handlers = useMemo<McpAppHostHandlers>(
    () => ({
      callTool: canCallTools
        ? async (name, args) => {
            const request = { server: app.server, name, arguments: args, confirmed: false };
            let response = await callMcpAppTool(sessionId, request);
            if (response.status === "confirmation_required") {
              const confirmed = await askToRun(response.title, response.description);
              if (!confirmed) {
                return { content: [{ type: "text", text: "The person declined." }], isError: true };
              }
              response = await callMcpAppTool(sessionId, { ...request, confirmed: true });
            }
            if (response.status !== "done") throw new Error("Tool call was not run");
            return response.result;
          }
        : undefined,
      readResource: canCallTools
        ? (uri) => readMcpAppResource(sessionId, { server: app.server, uri })
        : undefined,
      openLink: (url) => {
        window.open(url, "_blank", "noopener,noreferrer");
      },
      message: (text) => {
        window.dispatchEvent(
          new CustomEvent<ComposeDraftDetail>(COMPOSE_DRAFT_EVENT, { detail: { sessionId, text } }),
        );
      },
      requestDisplayMode: (next) => {
        setMode(next);
        return next;
      },
    }),
    [app.server, askToRun, canCallTools, sessionId],
  );

  if (view.data && view.data.interactive_views === "off") return null;

  const failed = view.isError || (stalled && !ready);
  const errorText = view.isError
    ? view.error instanceof Error
      ? view.error.message
      : ui("The view could not be loaded")
    : failed
      ? ui("the view did not start")
      : null;
  const textResult = callResult.content as Array<{ type: string; text?: string }>;

  const header = (expanded: boolean) => (
    <div className="flex items-center justify-between gap-3 border-b border-border/60 px-3 py-1.5 text-xs text-muted-foreground">
      <div className="flex min-w-0 items-center gap-1.5">
        <AppWindow className="h-3.5 w-3.5 shrink-0" />
        <span className="font-medium text-foreground">{label}</span>
        <span className="shrink-0">·</span>
        <span className="truncate">{app.tool}</span>
        <span className="hidden shrink-0 rounded-full border border-border/70 px-1.5 py-px text-[11px] sm:inline">
          {ui("interactive view from {value0}", { value0: label })}
        </span>
      </div>
      <div className="flex shrink-0 items-center gap-3">
        {failed ? (
          <button
            type="button"
            className="inline-flex items-center gap-1 hover:text-foreground"
            onClick={() => {
              setStalledLoad(null);
              void view.refetch();
            }}
          >
            <RotateCw className="h-3 w-3" />
            {ui("Retry view")}
          </button>
        ) : (
          !expanded && (
            <button
              type="button"
              className="inline-flex items-center gap-1 hover:text-foreground"
              onClick={() => setMode("fullscreen")}
            >
              <Maximize2 className="h-3 w-3" />
              {ui("Expand")}
            </button>
          )
        )}
        <button
          type="button"
          className="hover:text-foreground"
          aria-expanded={showData}
          onClick={() => setShowData((current) => !current)}
        >
          {showData ? ui("Hide data") : ui("Show data")}
        </button>
      </div>
    </div>
  );

  const data = showData && (
    <pre className="max-h-64 overflow-auto border-t border-border/60 px-3 py-2 font-mono text-[11px] leading-relaxed text-muted-foreground">
      {JSON.stringify(
        app.structured_content ?? textResult.map((part) => part.text ?? "").join("\n"),
        null,
        2,
      )}
    </pre>
  );

  const frame = (fullscreen: boolean) =>
    view.data && (
      <McpAppFrame
        key={view.dataUpdatedAt}
        html={view.data.html}
        policy={view.data.policy}
        tool={{ name: app.tool }}
        toolCallId={toolCallId}
        toolInput={toolInput}
        toolResult={callResult}
        displayMode={fullscreen ? "fullscreen" : "inline"}
        theme={theme === "dark" ? "dark" : "light"}
        locale={locale}
        handlers={handlers}
        title={ui("{value0} view: {value1}", { value0: label, value1: app.tool })}
        height={fullscreen ? "100%" : height}
        className={cn(!ready && !fullscreen && "pointer-events-none absolute inset-0 opacity-0")}
        onReady={() => setReady(true)}
        onHeight={setHeight}
      />
    );

  return (
    <div
      className="mt-2 overflow-hidden rounded-md border border-border/70 bg-background"
      data-testid="mcp-app-view"
    >
      {header(false)}
      {failed ? (
        <div className="px-3 py-2 text-[13px]">
          <div className="mb-1 text-xs text-warning">
            {ui("The interactive view didn't load ({value0}). Showing the text result.", {
              value0: errorText,
            })}
          </div>
          <div className="whitespace-pre-wrap text-foreground/90">
            {textResult.map((part) => part.text ?? "").join("\n")}
          </div>
        </div>
      ) : (
        <div className="relative">
          {!ready && (
            <div className="space-y-2 p-3" data-testid="mcp-app-view-loading">
              <Skeleton className="h-4 w-3/5" />
              <Skeleton className="h-36 w-full" />
              <div className="text-xs text-muted-foreground">
                {ui("Loading view from {value0}…", { value0: label })}
              </div>
            </div>
          )}
          {mode === "inline" && frame(false)}
        </div>
      )}
      {data}

      <Dialog
        open={mode === "fullscreen" && !failed}
        onOpenChange={(open) => !open && setMode("inline")}
      >
        <DialogContent className="flex h-[calc(100dvh-4rem)] w-[calc(100vw-4rem)] max-w-none flex-col gap-0 overflow-hidden p-0 sm:max-w-none">
          <DialogTitle className="sr-only">
            {ui("{value0} view: {value1}", { value0: label, value1: app.tool })}
          </DialogTitle>
          {header(true)}
          <div className="min-h-0 flex-1">{frame(true)}</div>
        </DialogContent>
      </Dialog>

      <ConfirmDialog
        open={confirm !== null}
        onOpenChange={(open) => {
          if (!open && confirm) {
            confirm.resolve(false);
            setConfirm(null);
          }
        }}
        title={ui("Let the {value0} view run {value1}?", {
          value0: label,
          value1: confirm?.title ?? "",
        })}
        description={
          confirm?.description ??
          ui("The server does not mark this tool as read-only, so it may change something.")
        }
        confirmLabel={ui("Run tool")}
        destructive={false}
        onConfirm={async () => {
          confirm?.resolve(true);
          setConfirm(null);
        }}
      />
    </div>
  );
}
