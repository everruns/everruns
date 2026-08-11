"use client";

import { useState, type ComponentType } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import type { Agent, ModelWithProvider, Session, SessionStatus, TokenUsage } from "@/lib/api/types";
import { buttonVariants } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { EntityIdentity } from "@/components/ui/entity-identity";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuPositioner,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { IconTile } from "@/components/layout/page-layout";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { useForkSession } from "@/hooks/use-sessions";
import { downloadSessionExport } from "@/lib/session-export";
import { useLocale } from "@/providers/locale-provider";
import { useOptionalNotificationsContext } from "@/providers/notifications-provider";
import {
  getDisplayName,
  getEntityReferenceClassName,
  getEntityReferenceLabel,
} from "@/lib/entity-lifecycle";
import { formatTokens } from "@/lib/formatting";
import { cn, shortenId } from "@/lib/utils";
import {
  Activity,
  ArrowLeft,
  Bot,
  Coins,
  Download,
  ExternalLink,
  Folder,
  GitFork,
  ListTree,
  Loader2,
  MessageSquare,
  Sparkles,
  Waypoints,
  Zap,
} from "lucide-react";

/**
 * A session is a recording, not a workspace, so its tabs are the views a
 * recording actually has. `files` keeps its route id — the label is
 * "Workspace" and renaming the route would break existing links.
 */
export type SessionNavKey = "transcript" | "timeline" | "work" | "events" | "files" | "cost";

export interface SessionNavItem {
  key: SessionNavKey;
  label: string;
  href: string;
  icon: ComponentType<{ className?: string }>;
  badge?: string;
}

export function SessionUsageBadge({ usage }: { usage: TokenUsage }) {
  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger>
          <Badge variant="outline" className="cursor-help gap-1">
            <Zap className="icon-sharp h-3 w-3" />
            {formatTokens(usage.input_tokens)} / {formatTokens(usage.output_tokens)}
          </Badge>
        </TooltipTrigger>
        <TooltipContent>
          <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
            <dt className="text-muted-foreground">Input</dt>
            <dd>{usage.input_tokens.toLocaleString()}</dd>
            <dt className="text-muted-foreground">Output</dt>
            <dd>{usage.output_tokens.toLocaleString()}</dd>
            {(usage.cache_read_tokens ?? 0) > 0 && (
              <>
                <dt className="text-muted-foreground">Cache read</dt>
                <dd>{usage.cache_read_tokens!.toLocaleString()}</dd>
              </>
            )}
            {(usage.cache_creation_tokens ?? 0) > 0 && (
              <>
                <dt className="text-muted-foreground">Cache created</dt>
                <dd>{usage.cache_creation_tokens!.toLocaleString()}</dd>
              </>
            )}
          </dl>
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}

/**
 * Link to the session's grouped trace on its provider's observability dashboard
 * (e.g. OpenRouter Logs). The URL and label are resolved by the caller (which
 * has org/provider context) and passed in, mirroring the `liveUsage` pattern, so
 * the header stays renderable in isolation (dev page, unit tests).
 */
function SessionTraceBadge({ href, label }: { href?: string; label?: string }) {
  if (!href) return null;

  return (
    <a
      href={href}
      target="_blank"
      rel="noopener noreferrer"
      className={cn(
        buttonVariants({ variant: "outline", size: "sm" }),
        "gap-1 text-muted-foreground",
      )}
      title={label}
    >
      <ExternalLink className="icon-sharp h-3 w-3" />
      {label}
    </a>
  );
}

export function SessionStatusBadge({ status }: { status: SessionStatus | undefined }) {
  if (status === "active") {
    return <Badge variant="default">Processing...</Badge>;
  }

  if (status === "idle") {
    return <Badge variant="secondary">Ready</Badge>;
  }

  if (status === "waiting_for_tool_results") {
    return <Badge variant="outline">Waiting on tools</Badge>;
  }

  if (status === "started") {
    return <Badge variant="outline">New</Badge>;
  }

  return null;
}

export function buildSessionNavigation({
  basePath,
  features,
  activeScheduleCount,
}: {
  basePath: string;
  features: Set<string>;
  activeScheduleCount?: number;
}): SessionNavItem[] {
  const hasFeature = (feature: string) => features.has(feature);
  // Work covers subagents, leased resources and the schedules this session
  // created, so either capability is enough to make the tab worth showing.
  const hasWork = hasFeature("leased_resources") || hasFeature("schedules");

  return [
    {
      key: "transcript",
      label: "Transcript",
      href: `${basePath}/transcript`,
      icon: MessageSquare,
    },
    {
      key: "timeline",
      label: "Timeline",
      href: `${basePath}/timeline`,
      icon: ListTree,
    },
    ...(hasWork
      ? [
          {
            key: "work" as const,
            label: "Work",
            href: `${basePath}/work`,
            icon: Waypoints,
            badge:
              activeScheduleCount && activeScheduleCount > 0
                ? String(activeScheduleCount)
                : undefined,
          },
        ]
      : []),
    {
      key: "events",
      label: "Events",
      href: `${basePath}/events`,
      icon: Activity,
    },
    ...(hasFeature("file_system")
      ? [
          {
            key: "files" as const,
            // Route stays /files — renaming it would break existing links.
            label: "Workspace",
            href: `${basePath}/files`,
            icon: Folder,
          },
        ]
      : []),
    {
      key: "cost",
      label: "Cost",
      href: `${basePath}/cost`,
      icon: Coins,
    },
  ];
}

/**
 * The escape hatches from a read-only recording (EVE-854): fork it into a chat
 * thread you can talk to, open the agent that ran it, or export the transcript.
 * Fork is the only request this page can issue, and it creates a new session
 * rather than changing this one.
 */
function SessionRecordingActions({
  sessionId,
  agentId,
  sessionTitle,
}: {
  sessionId: string;
  agentId?: string;
  sessionTitle: string | null;
}) {
  const router = useRouter();
  const { locale } = useLocale();
  const notificationsContext = useOptionalNotificationsContext();
  const forkSession = useForkSession();
  const [forkError, setForkError] = useState<string | null>(null);

  const handleFork = () => {
    setForkError(null);
    forkSession.mutate(
      {
        sessionId,
        // Title is the only override worth setting: agent, model, locale, goal
        // and tags should all inherit so the fork is the same run continued.
        request: { title: sessionTitle ? `${sessionTitle} (chat)` : undefined },
      },
      {
        onSuccess: (session) => router.push(`/chats/${session.id}`),
        onError: (error) =>
          setForkError(error instanceof Error ? error.message : "Could not fork this session"),
      },
    );
  };

  return (
    <>
      <button
        type="button"
        onClick={handleFork}
        disabled={forkSession.isPending}
        className={cn(buttonVariants({ variant: "default", size: "sm" }), "gap-1")}
      >
        {forkSession.isPending ? (
          <Loader2 className="icon-sharp h-4 w-4 animate-spin" />
        ) : (
          <GitFork className="icon-sharp h-4 w-4" />
        )}
        Fork into chat
      </button>

      {agentId && (
        <Link
          href={`/agents/${agentId}`}
          className={cn(buttonVariants({ variant: "outline", size: "sm" }), "gap-1")}
        >
          <Bot className="icon-sharp h-4 w-4" />
          Open agent
        </Link>
      )}

      <DropdownMenu>
        <DropdownMenuTrigger
          className={cn(buttonVariants({ variant: "outline", size: "sm" }), "gap-1")}
          aria-label="Export session"
        >
          <Download className="icon-sharp h-4 w-4" />
          Export
        </DropdownMenuTrigger>
        <DropdownMenuPositioner align="end">
          <DropdownMenuContent className="w-40">
            <DropdownMenuItem
              onClick={() =>
                void downloadSessionExport(sessionId, "jsonl", locale, notificationsContext?.notify)
              }
            >
              JSONL
            </DropdownMenuItem>
            <DropdownMenuItem
              onClick={() =>
                void downloadSessionExport(sessionId, "atif", locale, notificationsContext?.notify)
              }
            >
              ATIF
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenuPositioner>
      </DropdownMenu>

      {forkError && (
        <span role="alert" className="text-xs text-destructive">
          {forkError}
        </span>
      )}
    </>
  );
}

export function SessionHeader({
  sessionId,
  session,
  agent,
  agentId,
  llmModel,
  effectiveStatus,
  liveUsage,
  activeTab,
  navigationItems,
  secondaryMetaText,
  sessionTraceUrl,
  sessionTraceLabel,
  backHref = "/sessions",
  backLabel = "Back to Sessions",
}: {
  sessionId: string;
  session: Session;
  agent?: Agent;
  agentId?: string;
  llmModel?: ModelWithProvider;
  effectiveStatus?: SessionStatus;
  liveUsage?: TokenUsage;
  activeTab: SessionNavKey;
  navigationItems: SessionNavItem[];
  secondaryMetaText: string;
  /** Deep link to the session's provider trace, resolved by the caller. */
  sessionTraceUrl?: string;
  sessionTraceLabel?: string;
  backHref?: string;
  backLabel?: string;
}) {
  const agentReferenceLabel =
    session.agent_id != null
      ? getEntityReferenceLabel({
          kind: "Agent",
          name: getDisplayName(agent),
          status: agent?.status ?? "deleted",
        })
      : null;
  const agentReferenceStatus = session.agent_id != null ? (agent?.status ?? "deleted") : null;
  const getNavigationClassName = (isActive: boolean) =>
    cn(
      buttonVariants({ variant: "outline", size: "sm" }),
      isActive ? "border-primary bg-card text-foreground" : "text-muted-foreground",
    );

  return (
    <div className="border-b border-border/70 bg-background/80 px-4 py-3 backdrop-blur-[1px]">
      <Link
        href={backHref}
        className="mb-1.5 inline-flex items-center text-sm text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="icon-sharp mr-2 h-4 w-4" />
        {backLabel}
      </Link>

      <div className="flex items-center justify-between gap-4">
        <div className="flex min-w-0 items-start gap-3">
          <IconTile size="md" icon={<MessageSquare />} className="mt-0.5" />
          <div className="min-w-0">
            <h1 className="flex min-w-0 flex-wrap items-center gap-2 text-xl font-bold">
              <EntityIdentity value={sessionId}>
                {session.title || `Session ${shortenId(session.id)}`}
              </EntityIdentity>
            </h1>
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              {agentReferenceLabel &&
                (agent && agentId ? (
                  <Link
                    href={`/agents/${agentId}`}
                    className="inline-flex items-center gap-1 hover:text-foreground"
                  >
                    <Bot className="icon-sharp h-3 w-3" />
                    <span className={getEntityReferenceClassName(agentReferenceStatus)}>
                      {agentReferenceLabel}
                    </span>
                  </Link>
                ) : (
                  <span className="inline-flex items-center gap-1">
                    <Bot className="icon-sharp h-3 w-3" />
                    <span className={getEntityReferenceClassName(agentReferenceStatus)}>
                      {agentReferenceLabel}
                    </span>
                  </span>
                ))}
              <span>•</span>
              <span>{secondaryMetaText}</span>
            </div>
          </div>
        </div>

        <div className="flex flex-wrap items-center justify-end gap-2">
          {liveUsage && <SessionUsageBadge usage={liveUsage} />}

          {llmModel && (
            <Badge variant="outline" className="gap-1">
              <Sparkles className="icon-sharp h-3 w-3" />
              {llmModel.display_name}
            </Badge>
          )}

          <SessionTraceBadge href={sessionTraceUrl} label={sessionTraceLabel} />

          <SessionStatusBadge status={effectiveStatus} />

          <SessionRecordingActions
            sessionId={sessionId}
            agentId={agent && agentId ? agentId : undefined}
            sessionTitle={session.title ?? null}
          />
        </div>
      </div>

      {/* Tabs get their own row: five of them plus the escape hatches do not fit
          on one line, and crowding them squeezes the title out of legibility. */}
      <nav className="mt-3 flex flex-wrap items-center gap-2">
        {navigationItems.map((item) => (
          <Link
            key={item.key}
            href={item.href}
            className={getNavigationClassName(activeTab === item.key)}
            aria-current={activeTab === item.key ? "page" : undefined}
          >
            <item.icon className="icon-sharp h-4 w-4" />
            {item.label}
            {item.badge && <span className="text-xs text-muted-foreground">{item.badge}</span>}
          </Link>
        ))}
      </nav>
    </div>
  );
}
