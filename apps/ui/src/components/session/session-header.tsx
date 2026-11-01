"use client";

import {
  useLocale as useDisplayLocale,
  useLocale,
  useUiTranslations,
} from "@/providers/locale-provider";

import { SessionCredentialsDialog } from "@/components/credentials/session-credentials-dialog";
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

import { useOptionalNotificationsContext } from "@/providers/notifications-provider";
import {
  getDisplayName,
  getEntityReferenceClassName,
  getEntityReferenceLabel,
} from "@/lib/entity-lifecycle";
import { formatCompactNumber, formatCostUsd, formatTokens } from "@/lib/formatting";
import { cn, shortenId } from "@/lib/utils";
import { sessionOutcome } from "@/lib/session-outcome";
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
  const { displayLocale } = useLocale();
  const cost = usage.effective_cost_usd ?? usage.actual_cost_usd ?? usage.estimated_cost_usd;

  const ui = useUiTranslations();
  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger>
          <Badge variant="outline" className="cursor-help gap-1">
            <Zap className="icon-sharp h-3 w-3" />
            {cost != null ? `${formatCostUsd(cost)} · ` : ""}
            {formatTokens(usage.input_tokens)} / {formatTokens(usage.output_tokens)}
          </Badge>
        </TooltipTrigger>
        <TooltipContent>
          <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
            <dt className="text-muted-foreground">{ui("Input")}</dt>
            <dd>{usage.input_tokens.toLocaleString(displayLocale)}</dd>
            <dt className="text-muted-foreground">{ui("Output")}</dt>
            <dd>{usage.output_tokens.toLocaleString(displayLocale)}</dd>
            {(usage.cache_read_tokens ?? 0) > 0 && (
              <>
                <dt className="text-muted-foreground">{ui("Cache read")}</dt>
                <dd>{usage.cache_read_tokens!.toLocaleString(displayLocale)}</dd>
              </>
            )}
            {(usage.cache_creation_tokens ?? 0) > 0 && (
              <>
                <dt className="text-muted-foreground">{ui("Cache created")}</dt>
                <dd>{usage.cache_creation_tokens!.toLocaleString(displayLocale)}</dd>
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

export function SessionStatusBadge({
  status,
  session,
}: {
  status: SessionStatus | undefined;
  session?: Session;
}) {
  const outcome = sessionOutcome(session ?? { status: status ?? "started" }, status);
  return (
    <Badge variant={outcome.variant} className={outcome.className}>
      {outcome.label}
    </Badge>
  );
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

/** Filesystem, sandbox files, stored values, or schedules — anything the Files tab can show. */
export function sessionShowsFiles(features: Set<string>, fileCount?: number): boolean {
  return (
    features.has("file_system") ||
    features.has("managed_sandbox") ||
    features.has("key_value") ||
    features.has("secrets") ||
    features.has("schedules") ||
    (fileCount ?? 0) > 0
  );
}

export type SessionRecordPane = "files" | "values" | "schedules";

/**
 * Panes on the Files tab. The filesystem is always there so a recording can be
 * inspected even when the agent did not declare `file_system`. Key values and
 * schedules are separate panes, including values and schedules left behind
 * after a capability was removed.
 */
export function sessionRecordPanes({
  features,
  hasSchedules,
  hasValues,
}: {
  features: Set<string>;
  hasSchedules: boolean;
  hasValues?: boolean;
}): SessionRecordPane[] {
  const panes: SessionRecordPane[] = ["files"];
  if (hasValues || features.has("key_value") || features.has("secrets")) panes.push("values");
  if (hasSchedules || features.has("schedules")) panes.push("schedules");
  return panes;
}

export function buildSessionNavigation({
  basePath,
  features,
  taskCount,
  eventCount,
  fileCount,
  activeScheduleCount,
}: {
  basePath: string;
  features: Set<string>;
  /** Background work the session owns — subagents, external agents, background tools. */
  taskCount?: number;
  /** Every event in the session. */
  eventCount?: number;
  /** Non-directory files in the session's workspace. */
  fileCount?: number;
  /** Enabled schedules. A session that still has one keeps the Work tab. */
  activeScheduleCount?: number;
}): SessionNavItem[] {
  const hasFeature = (feature: string) => features.has(feature);
  // Work covers subagents, leased resources and the schedules this session
  // created. A count covers a recording whose capability was removed later.
  const hasWork =
    hasFeature("leased_resources") ||
    hasFeature("schedules") ||
    (taskCount ?? 0) > 0 ||
    (activeScheduleCount ?? 0) > 0;
  const hasFiles = sessionShowsFiles(features, fileCount);

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
    ...(hasFiles
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
  const ui = useUiTranslations();
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

      <SessionCredentialsDialog sessionId={sessionId} />
      <DropdownMenu>
        <DropdownMenuTrigger
          className={cn(buttonVariants({ variant: "outline", size: "sm" }), "gap-1")}
          aria-label={ui("Export session")}
        >
          <Download className="icon-sharp h-4 w-4" />
          {ui("Export")}
        </DropdownMenuTrigger>
        <DropdownMenuPositioner align="end">
          <DropdownMenuContent className="w-40">
            <DropdownMenuItem
              onClick={() =>
                void downloadSessionExport(sessionId, "jsonl", locale, notificationsContext?.notify)
              }
            >
              {ui("JSONL")}
            </DropdownMenuItem>
            <DropdownMenuItem
              onClick={() =>
                void downloadSessionExport(sessionId, "atif", locale, notificationsContext?.notify)
              }
            >
              {ui("ATIF")}
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
  const { displayLocale } = useDisplayLocale();
  const ui = useUiTranslations();
  const agentReferenceLabel =
    session.agent_id != null
      ? getEntityReferenceLabel({
          kind: "Agent",
          name: getDisplayName(agent, displayLocale),
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
          { label: session.title || ui("Session {value0}", { value0: shortenId(session.id) }) },
        ]}
      />

      <div className="flex flex-col items-start justify-between gap-4 sm:flex-row sm:items-center">
        <div className="flex min-w-0 items-start gap-3">
          <IconTile size="md" icon={<MessageSquare />} className="mt-0.5" />
          <div className="min-w-0">
            <h1 className="flex min-w-0 flex-wrap items-center gap-2 text-xl font-bold">
              <EntityIdentity value={sessionId}>
                {session.title || ui("Session {value0}", { value0: shortenId(session.id) })}
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

          <SessionStatusBadge status={effectiveStatus} session={session} />

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
        aria-label={ui("Session views")}
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
