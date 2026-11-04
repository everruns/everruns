"use client";
import { Pagination } from "@/components/ui/pagination";

import { useUiTranslations, useLocale } from "@/providers/locale-provider";
import { useMemo, useState } from "react";
import { useModels, useSessions } from "@/hooks";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { SessionCard } from "@/components/session/session-card";
import type { ModelWithProvider } from "@/lib/api/types";
import { useRouter, useSearchParams } from "next/navigation";
import { useAgentFailures } from "@/hooks/use-agent-failures";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

const PAGE_SIZE = 20;

/** The agent page's Sessions tab: every session of the agent, paginated. */
export function AgentSessionsPanel({
  agentId,
  defaultModelId,
}: {
  agentId: string;
  defaultModelId?: string | null;
}) {
  const ui = useUiTranslations();
  const params = useSearchParams();
  const router = useRouter();
  const failures = params.get("failures") === "true";
  return (
    <section aria-label={ui("Agent sessions")} className="space-y-4">
      <div className="flex items-center gap-2">
        {[false, true].map((value) => (
          <Button
            key={String(value)}
            variant={failures === value ? "secondary" : "outline"}
            aria-pressed={failures === value}
            onClick={() => {
              const next = new URLSearchParams(params.toString());
              if (value) next.set("failures", "true");
              else {
                next.delete("failures");
                next.delete("after");
                next.delete("before");
              }
              router.replace(`/agents/${agentId}?${next}`, { scroll: false });
            }}
          >
            {value ? ui("With failed turns") : ui("All sessions")}
          </Button>
        ))}
      </div>
      {failures ? (
        <FailedSessions agentId={agentId} />
      ) : (
        <AllSessions agentId={agentId} defaultModelId={defaultModelId} />
      )}
    </section>
  );
}

function FailedSessions({ agentId }: { agentId: string }) {
  const params = useSearchParams();
  const router = useRouter();
  const [anchor] = useState(() => new Date().toISOString());
  const before = params.get("before") || anchor;
  const after =
    params.get("after") ||
    new Date(
      (Number.isFinite(Date.parse(before)) ? Date.parse(before) : Date.parse(anchor)) - 86400000,
    ).toISOString();
  const windowKey = `${after}:${before}`;
  return (
    <FailedSessionsWindow
      key={windowKey}
      agentId={agentId}
      after={after}
      before={before}
      onWindow={(days) => {
        const end = new Date();
        const next = new URLSearchParams(params.toString());
        next.set("before", end.toISOString());
        next.set("after", new Date(end.getTime() - days * 86400000).toISOString());
        router.replace(`/agents/${agentId}?${next}`, { scroll: false });
      }}
    />
  );
}

function FailedSessionsWindow({
  agentId,
  after,
  before,
  onWindow,
}: {
  agentId: string;
  after: string;
  before: string;
  onWindow: (days: number) => void;
}) {
  const { displayLocale } = useLocale();

  const ui = useUiTranslations();
  const [page, onPage] = useState(0);
  const { data, isLoading, error, refetch } = useAgentFailures(
    agentId,
    after,
    before,
    page * PAGE_SIZE,
  );
  const duration = (Date.parse(before) - Date.parse(after)) / 86400000;
  const days = [1, 7, 30].find((value) => Math.abs(duration - value) < 0.001);
  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="text-sm text-muted-foreground">
          {data && (
            <p>
              {data.failed_turns} {ui("failed")}{" "}
              {data.failed_turns === 1 ? ui("turn") : ui("turns")} {ui("in")} {data.total_sessions}{" "}
              {data.total_sessions === 1 ? ui("session") : ui("sessions")}
            </p>
          )}
          <p className="mt-1 text-xs">
            {new Date(after).toLocaleString(displayLocale)} –{" "}
            {new Date(before).toLocaleString(displayLocale)}
          </p>
        </div>
        <Select
          value={days ? String(days) : "custom"}
          onValueChange={(value) => {
            if (value && value !== "custom") onWindow(Number(value));
          }}
        >
          <SelectTrigger className="w-52" aria-label={ui("Failure time window")}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="1">{ui("Last 24 hours")}</SelectItem>
            <SelectItem value="7">{ui("Last 7 days")}</SelectItem>
            <SelectItem value="30">{ui("Last 30 days")}</SelectItem>
            {!days && <SelectItem value="custom">{ui("Selected window")}</SelectItem>}
          </SelectContent>
        </Select>
      </div>
      {isLoading ? (
        <Skeleton className="h-24 w-full" />
      ) : error ? (
        <div className="border p-4 text-sm">
          <p>{ui("Could not load failed turns.")}</p>
          <Button variant="outline" onClick={() => void refetch()}>
            {ui("Try again")}
          </Button>
        </div>
      ) : (
        <>
          {data?.data.length === 0 && (
            <p className="border p-8 text-center text-sm text-muted-foreground">
              {ui("No failed turns in this window.")}
            </p>
          )}
          {data?.data.map((item) => (
            <SessionCard
              key={item.session.id}
              session={item.session}
              failure={{
                count: item.failed_turns,
                at: item.failed_at,
                error: item.error,
                sequence: item.failure_sequence,
              }}
            />
          ))}
          {data && data.total_sessions > PAGE_SIZE && (
            <div className="flex items-center justify-between">
              <Button variant="outline" disabled={page === 0} onClick={() => onPage(page - 1)}>
                {ui("Previous")}
              </Button>
              <span className="text-sm text-muted-foreground">
                {ui("Page")} {page + 1} {ui("of")} {Math.ceil(data.total_sessions / PAGE_SIZE)}
              </span>
              <Button
                variant="outline"
                disabled={(page + 1) * PAGE_SIZE >= data.total_sessions}
                onClick={() => onPage(page + 1)}
              >
                {ui("Next")}
              </Button>
            </div>
          )}
        </>
      )}
    </div>
  );
}

function AllSessions({
  agentId,
  defaultModelId,
}: {
  agentId: string;
  /** Model a session without its own model runs on: the agent's default. */
  defaultModelId?: string | null;
}) {
  const ui = useUiTranslations();
  const [page, setPage] = useState(0);
  const offset = page * PAGE_SIZE;
  const { data: sessionsResponse, isLoading } = useSessions(agentId, {
    offset,
    limit: PAGE_SIZE,
    // The agent's session_count includes archived sessions too.
    includeArchived: true,
  });
  const { data: models } = useModels();
  const sessions = sessionsResponse?.data ?? [];
  const totalSessions = sessionsResponse?.total ?? 0;
  const totalPages = Math.ceil(totalSessions / PAGE_SIZE);

  const modelMap = useMemo(
    () => new Map<string, ModelWithProvider>((models ?? []).map((m) => [m.id, m])),
    [models],
  );

  if (isLoading) {
    return (
      <div className="space-y-2">
        {Array.from({ length: 4 }).map((_, i) => (
          <Skeleton key={i} className="h-16 w-full" />
        ))}
      </div>
    );
  }

  if (sessions.length === 0) {
    return (
      <p className="border bg-background py-10 text-center text-sm text-muted-foreground">
        {ui("No sessions yet. Start a test chat from the agent page to begin.")}
      </p>
    );
  }

  return (
    <div className="space-y-3">
      <p className="text-sm text-muted-foreground">
        {totalSessions} {totalSessions === 1 ? ui("session") : ui("sessions")}{" "}
        {ui("· Includes archived sessions")}
      </p>
      <div className="space-y-2">
        {sessions.map((session) => (
          <SessionCard
            key={session.id}
            session={session}
            model={modelMap.get(session.model_id ?? defaultModelId ?? "")}
          />
        ))}
      </div>

      <Pagination
        page={page}
        totalPages={totalPages}
        onPageChange={setPage}
        className="border-t pt-3"
        summary={`${ui("Showing")} ${offset + 1}-${Math.min(offset + PAGE_SIZE, totalSessions)} ${ui("of")} ${totalSessions} ${ui("sessions")}`}
      />
    </div>
  );
}
