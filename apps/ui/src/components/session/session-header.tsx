"use client";

import { AgentIcon } from "@/components/icons/facet-icons";
import type { ComponentType } from "react";
import Link from "next/link";
import type { Agent, ModelWithProvider, Session, SessionStatus, TokenUsage } from "@/lib/api/types";
import { buttonVariants } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { EntityIdentity } from "@/components/ui/entity-identity";
import { SessionForkButton } from "@/components/session/session-fork-button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuPositioner,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { IconTile, PageBreadcrumb, SectionTabs } from "@/components/layout/page-layout";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { downloadSessionExport } from "@/lib/session-export";
import { useLocale } from "@/providers/locale-provider";
import { useOptionalNotificationsContext } from "@/providers/notifications-provider";
import {
  getDisplayName,
  getEntityReferenceClassName,
  getEntityReferenceLabel,
} from "@/lib/entity-lifecycle";
import { formatCompactNumber, formatTokens } from "@/lib/formatting";
import { cn, shortenId } from "@/lib/utils";
import {
  Activity,
  Coins,
  Download,
  ExternalLink,
  Folder,
  MessageSquare,
  ShieldCheck,
  Sparkles,
  Waypoints,
  Workflow,
  Zap,
} from "lucide-react";

/**
 * A session is a recording, not a workspace, so its tabs are the views a
 * recording actually has. `files` keeps its route id — the label is
 * "Files"; its stable route preserves existing links.
 */
export type SessionNavKey = "trace" | "approvals" | "work" | "events" | "files" | "cost";

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
      <ExternalLink className="icon-sharp h-4 w-4" />
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

/**
 * A tab badge (EVE-868). Zero renders as no badge at all: an empty tab should
 * look empty, not annotated with a `0`. Counts are compacted so a 12,000-event
 * recording does not stretch the tab bar.
 *
 * The server omits a count it could not cheaply produce, so `undefined` means
 * "unknown", which reads the same as "nothing here" — an absent badge is
 * honest either way.
 */
function tabBadge(count: number | undefined): string | undefined {
  if (!count || count <= 0) return undefined;
  return formatCompactNumber(count);
}

export function buildSessionNavigation({
  basePath,
  features,
  taskCount,
  eventCount,
  fileCount,
}: {
  basePath: string;
  features: Set<string>;
  /** Background work the session owns — subagents, external agents, background tools. */
  taskCount?: number;
  /** Every event in the session. */
  eventCount?: number;
  /** Non-directory files in the session's workspace. */
  fileCount?: number;
}): SessionNavItem[] {
  const hasFeature = (feature: string) => features.has(feature);
  // Work covers subagents, leased resources and the schedules this session
  // created, so either capability is enough to make the tab worth showing.
  const hasWork = hasFeature("leased_resources") || hasFeature("schedules");

  return [
    {
      key: "trace",
      label: "Trace",
      href: `${basePath}/trace`,
      icon: Workflow,
    },
    // Approvals pairs each request with the grant recorded for it. The count
    // is not a denormalized session counter, so the tab stays unbadged rather
    // than scanning the event log to paint a number.
    {
      key: "approvals",
      label: "Approvals",
      href: `${basePath}/approvals`,
      icon: ShieldCheck,
    },
    ...(hasWork
      ? [
          {
            key: "work" as const,
            label: "Work",
            href: `${basePath}/work`,
            icon: Waypoints,
            // Tasks, not schedules: the tab holds subagents and background work
            // as well, and a schedule-only count undercounts what is behind it.
            badge: tabBadge(taskCount),
          },
        ]
      : []),
    {
      key: "events",
      label: "Events",
      href: `${basePath}/events`,
      icon: Activity,
      badge: tabBadge(eventCount),
    },
    ...(hasFeature("file_system")
      ? [
          {
            key: "files" as const,
            // Route stays /files — renaming it would break existing links.
            label: "Files",
            href: `${basePath}/files`,
            icon: Folder,
            badge: tabBadge(fileCount),
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
 * thread you can talk to, test its agent in Playground, or export the transcript.
 * Fork is the only request this page can issue, and it creates a new session
 * rather than changing this one.
 */
function SessionRecordingActions({
  sessionId,
  agentId,
  sessionTitle,
  sessionTags,
  platformChat,
}: {
  sessionId: string;
  agentId?: string;
  sessionTitle: string | null;
  sessionTags: string[];
  platformChat: boolean;
}) {
  const { locale } = useLocale();
  const notificationsContext = useOptionalNotificationsContext();

  return (
    <>
      <SessionForkButton
        sessionId={sessionId}
        sessionTitle={sessionTitle}
        sessionTags={sessionTags}
        agentId={agentId}
        platformChat={platformChat}
      />

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
  backLabel = "Sessions",
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
  return (
    <div className="border-b border-border/70 bg-background/80 px-4 py-3 backdrop-blur-[1px]">
      {/* EVE-869: the breadcrumb names the owning group and keeps the way back —
          `backLabel` stays navigable to the list, so nothing is lost by dropping
          the separate back link. */}
      <PageBreadcrumb
        className="mb-1.5"
        items={[
          { label: backLabel, href: backHref },
          { label: session.title || `Session ${shortenId(session.id)}` },
        ]}
      />

      <div className="flex flex-col items-start justify-between gap-4 sm:flex-row sm:items-center">
        <div className="flex min-w-0 items-start gap-3">
          <IconTile size="md" icon={<MessageSquare />} className="mt-0.5" />
          <div className="min-w-0">
            <h1 className="flex min-w-0 flex-wrap items-center gap-2 text-xl font-bold">
              <EntityIdentity value={sessionId}>
                {session.title || `Session ${shortenId(session.id)}`}
              </EntityIdentity>
            </h1>
            <div className="flex flex-wrap items-center gap-2 text-sm text-muted-foreground">
              {agentReferenceLabel &&
                (agent && agentId ? (
                  <Link
                    href={`/agents/${agentId}`}
                    className="inline-flex items-center gap-1 hover:text-foreground"
                  >
                    <AgentIcon className="icon-sharp h-3 w-3" />
                    <span className={getEntityReferenceClassName(agentReferenceStatus)}>
                      {agentReferenceLabel}
                    </span>
                  </Link>
                ) : (
                  <span className="inline-flex items-center gap-1">
                    <AgentIcon className="icon-sharp h-3 w-3" />
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

        <div className="flex flex-wrap items-center gap-2 sm:justify-end">
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
            sessionTags={session.tags ?? []}
            platformChat={agent?.name === "platform-chat"}
          />
        </div>
      </div>

      {/* Tabs get their own row: five of them plus the escape hatches do not fit
          on one line, and crowding them squeezes the title out of legibility. */}
      <SectionTabs
        value={activeTab}
        aria-label="Session views"
        className="mt-3"
        items={navigationItems.map((item) => ({
          value: item.key,
          label: item.label,
          href: item.href,
          icon: <item.icon className="icon-sharp size-4" />,
          count: item.badge,
        }))}
      />
    </div>
  );
}
