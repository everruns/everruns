"use client";

import {
  useLocale as useDisplayLocale,
  useUiTranslations,
  useLocale,
} from "@/providers/locale-provider";

import { useMemo } from "react";
import { usePathname } from "next/navigation";
import { ResourceNotFound } from "@/components/resource-not-found";
import { Skeleton } from "@/components/ui/skeleton";
import { useSessionContext } from "@/app/(main)/sessions/[sessionId]/session-context";
import {
  SessionHeader,
  type SessionNavKey,
  buildSessionNavigation,
} from "@/components/session/session-header";
import { usePageTitle } from "@/hooks";
import { useOptionalFeatureFlags } from "@/providers/feature-flags-provider";
import { useProviders } from "@/hooks/use-providers";
import { buildTraceConfigByDriver, resolveSessionTraceUrl } from "@/lib/chat-trace";
import { getDisplayName } from "@/lib/entity-lifecycle";

const SESSION_TAB_LABELS: Record<SessionNavKey, string> = {
  trace: "Trace",
  approvals: "Approvals",
  agents: "Agents",
  work: "Work",
  events: "Events",
  changes: "Changes",
  files: "Files",
  cost: "Cost",
};

interface SessionLayoutContentProps {
  credentialsAction?: React.ReactNode;
  onForked?: (sessionId: string) => void;
  children: React.ReactNode;
  sessionId: string;
}

export function SessionLayoutContent({
  children,
  sessionId,
  credentialsAction,
  onForked,
}: SessionLayoutContentProps) {
  const { displayLocale } = useDisplayLocale();
  const ui = useUiTranslations();
  const pathname = usePathname();
  const { agent, session, llmModel, sessionLoading, effectiveStatus, liveUsage, agentId } =
    useSessionContext();

  // Determine active tab from pathname
  const getActiveTab = (): SessionNavKey => {
    if (pathname.endsWith("/trace")) return "trace";
    if (pathname.endsWith("/changes")) return "changes";
    if (pathname.endsWith("/files")) return "files";
    if (pathname.endsWith("/events")) return "events";
    if (pathname.endsWith("/work")) return "work";
    if (pathname.endsWith("/agents")) return "agents";
    if (pathname.endsWith("/cost")) return "cost";
    if (pathname.endsWith("/approvals")) return "approvals";
    return "trace"; // Default while the base route and old tabs redirect.
  };
  const activeTab = getActiveTab();

  const sessionLabel = session?.title || (agent ? getDisplayName(agent, displayLocale) : null);
  usePageTitle(SESSION_TAB_LABELS[activeTab], sessionLabel, ui("Session"));

  const basePath = `/sessions/${sessionId}`;
  const features = useMemo(() => new Set(session?.features ?? []), [session?.features]);
  const flags = useOptionalFeatureFlags()?.flags;
  const navigationItems = buildSessionNavigation({
    basePath,
    features,
    taskCount: session?.task_count,
    eventCount: session?.event_count,
    fileCount: session?.file_count,
    activeScheduleCount: session?.active_schedule_count,
    inRun: !!session?.parent_session_id,
    dataPlane: !!flags?.data_plane,
  });

  // Resolve the session-level provider trace link here, where org/provider
  // context is available, and pass it to the (presentational) header.
  const { t } = useLocale();
  const { data: providers } = useProviders();
  const sessionTraceUrl = useMemo(() => {
    const traceByDriver = buildTraceConfigByDriver(providers);
    return (
      resolveSessionTraceUrl(llmModel?.provider_type, sessionId, traceByDriver, {
        model: llmModel?.model_id,
      }) ?? undefined
    );
  }, [providers, llmModel?.provider_type, llmModel?.model_id, sessionId]);

  if (sessionLoading) {
    return (
      <div className="container mx-auto p-6">
        <Skeleton className="h-8 w-1/3 mb-4" />
        <Skeleton className="h-4 w-2/3 mb-8" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  if (!session) {
    return (
      <ResourceNotFound
        title={ui("Session not found")}
        description={ui(
          "This session may have been deleted, moved to another organization, or the URL may be wrong.",
        )}
        backHref="/sessions"
        backLabel="Back to sessions"
        resourceId={sessionId}
      />
    );
  }

  return (
    <div className="flex flex-col h-full">
      <SessionHeader
        credentialsAction={credentialsAction}
        onForked={onForked}
        sessionId={sessionId}
        session={session}
        agent={agent}
        agentId={agentId}
        llmModel={llmModel}
        effectiveStatus={effectiveStatus}
        liveUsage={liveUsage}
        activeTab={activeTab}
        navigationItems={navigationItems}
        secondaryMetaText={
          activeTab === "files"
            ? "Current session files"
            : // EVE-867: the generated run summary explains what happened;
              // the start timestamp is metadata. Prefer the explanation where
              // one exists, and fall back for chat threads, runs that have not
              // finished, and deployments with no utility LLM.
              (session.run_summary?.trim() ??
              `Started ${new Date(session.created_at).toLocaleString(displayLocale)}`)
        }
        sessionTraceUrl={sessionTraceUrl}
        sessionTraceLabel={t("trace_view_session")}
      />

      {/* Tab content */}
      {children}
    </div>
  );
}
