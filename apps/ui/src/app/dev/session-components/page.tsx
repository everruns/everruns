"use client";

import { DevPageShell } from "@/app/dev/_components/dev-page-shell";
import {
  sessionCardScenarios,
  sessionHeaderScenarios,
  sessionShowcaseAgent,
  sessionShowcaseModel,
  sessionUsageSamples,
} from "@/app/dev/_fixtures/session-showcase-fixtures";
import {
  SessionHeader,
  SessionStatusBadge,
  SessionUsageBadge,
  buildSessionNavigation,
} from "@/components/session/session-header";
import { SessionApprovals } from "@/components/session/session-approvals";
import { SessionCard } from "@/components/session/session-card";
import { SessionSandboxPanel } from "@/components/session/session-sandbox-panel";
import { sessionSandboxScenarios } from "@/app/dev/_fixtures/session-sandbox-fixtures";
import { buildApprovalEpisodes } from "@/lib/approval-episodes";
import type { Event } from "@/lib/api/types";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

// The panel fetches its own data, so the showcase seeds the cache instead of
// mocking the client. Same component, same query key, no network.
function seededClient(sessionId: string, sandbox: unknown) {
  // staleTime keeps the seeded value from being refetched against an API this
  // page does not have.
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  });
  client.setQueryData(["session-sandbox", sessionId], sandbox);
  return client;
}

export default function DevSessionComponentsPage() {
  return (
    <DevPageShell
      eyebrow="Session UI"
      title="Session Components"
      description="Real session-page chrome and list components in the states the main UI uses."
      widthClassName="max-w-7xl"
    >
      <div className="space-y-6">
        <section className="space-y-4 border border-border/70 bg-card/90 p-4 shadow-[inset_0_1px_0_hsl(var(--background)/0.92)]">
          <div className="space-y-1">
            <h2 className="text-lg font-semibold text-foreground">Session header states</h2>
            <p className="text-sm text-muted-foreground">
              Extracted from the real session layout so the showcase matches main.
            </p>
          </div>

          <div className="space-y-4">
            {sessionHeaderScenarios.map((scenario) => (
              <div key={scenario.session.id} className="space-y-2">
                <p className="text-sm font-medium text-foreground">{scenario.name}</p>
                <div className="overflow-hidden border border-border/70 bg-background">
                  <SessionHeader
                    sessionId={scenario.session.id}
                    session={scenario.session}
                    agent={sessionShowcaseAgent}
                    agentId={sessionShowcaseAgent.id}
                    llmModel={sessionShowcaseModel}
                    effectiveStatus={scenario.effectiveStatus}
                    liveUsage={scenario.liveUsage}
                    activeTab={scenario.activeTab}
                    navigationItems={buildSessionNavigation({
                      basePath: `/sessions/${scenario.session.id}`,
                      features: new Set(scenario.session.features ?? []),
                      taskCount: scenario.session.task_count,
                      eventCount: scenario.session.event_count,
                      fileCount: scenario.session.file_count,
                    })}
                    secondaryMetaText={scenario.secondaryMetaText}
                  />
                </div>
              </div>
            ))}
          </div>
        </section>

        <section className="space-y-4 border border-border/70 bg-card/90 p-4 shadow-[inset_0_1px_0_hsl(var(--background)/0.92)]">
          <div className="space-y-1">
            <h2 className="text-lg font-semibold text-foreground">Header badges</h2>
            <p className="text-sm text-muted-foreground">
              These are the exact badge components used inside the session header.
            </p>
          </div>

          <div className="flex flex-wrap items-center gap-3">
            <SessionUsageBadge usage={sessionUsageSamples.light} />
            <SessionUsageBadge usage={sessionUsageSamples.medium} />
            <SessionStatusBadge status="started" />
            <SessionStatusBadge status="active" />
            <SessionStatusBadge status="idle" />
            <SessionStatusBadge status="waiting_for_tool_results" />
          </div>
        </section>

        <section className="space-y-4 border border-border/70 bg-card/90 p-4 shadow-[inset_0_1px_0_hsl(var(--background)/0.92)]">
          <div className="space-y-1">
            <h2 className="text-lg font-semibold text-foreground">Sandbox panel</h2>
            <p className="text-sm text-muted-foreground">
              The Workspace-tab panel, in the four sandbox states a session can actually have. The
              capability rows are the point: they answer &ldquo;can this session run a build?&rdquo;
              before a run proves it cannot.
            </p>
          </div>

          <div className="grid gap-3 lg:grid-cols-2">
            {sessionSandboxScenarios.map((scenario) => (
              <div key={scenario.name} className="space-y-2">
                <p className="text-sm font-medium text-foreground">{scenario.name}</p>
                <div className="overflow-hidden border border-border/70 bg-background">
                  <QueryClientProvider client={seededClient(scenario.sessionId, scenario.sandbox)}>
                    <SessionSandboxPanel sessionId={scenario.sessionId} />
                  </QueryClientProvider>
                </div>
              </div>
            ))}
          </div>
        </section>

        <section className="space-y-4 border border-border/70 bg-card/90 p-4 shadow-[inset_0_1px_0_hsl(var(--background)/0.92)]">
          <div className="space-y-1">
            <h2 className="text-lg font-semibold text-foreground">Session approvals</h2>
            <p className="text-sm text-muted-foreground">
              A request and the grant recorded for it, on one card. An open request and a grant with
              no logged request sit beside it.
            </p>
          </div>
          <div className="border border-border/70 bg-background p-4">
            <SessionApprovals
              sessionId="session_showcase_approvals"
              episodes={approvalShowcaseEpisodes()}
            />
          </div>
        </section>

        <section className="space-y-4 border border-border/70 bg-card/90 p-4 shadow-[inset_0_1px_0_hsl(var(--background)/0.92)]">
          <div className="space-y-1">
            <h2 className="text-lg font-semibold text-foreground">Session cards</h2>
            <p className="text-sm text-muted-foreground">
              The same list cards used on the sessions index, shown in running, idle, and new
              states.
            </p>
          </div>

          <div className="grid gap-3 lg:grid-cols-3">
            {sessionCardScenarios.map((scenario) => (
              <div key={scenario.session.id} className="overflow-hidden border border-border/70">
                <SessionCard
                  session={scenario.session}
                  agentName={sessionShowcaseAgent.display_name ?? sessionShowcaseAgent.name}
                  agentStatus={sessionShowcaseAgent.status}
                  model={sessionShowcaseModel}
                  summary={scenario.summary}
                />
              </div>
            ))}
          </div>
        </section>
      </div>
    </DevPageShell>
  );
}

function approvalToolEvent(
  id: string,
  toolName: string,
  payload: Record<string, unknown>,
  sequence: number,
): Event {
  return {
    id,
    type: "tool.completed",
    ts: `2026-10-02T22:56:${String(sequence).padStart(2, "0")}Z`,
    sequence,
    session_id: "session_showcase_approvals",
    context: {},
    data: {
      tool_call_id: id,
      tool_name: toolName,
      success: true,
      status: "success",
      result: [{ type: "text", text: JSON.stringify(payload) }],
    },
  };
}

function approvalShowcaseEpisodes() {
  const consent: Event = {
    id: "event_msg_consent",
    type: "input.message",
    ts: "2026-10-02T22:56:40Z",
    sequence: 3,
    session_id: "session_showcase_approvals",
    context: {},
    metadata: { initiator: { type: "user", user_id: "user_mykhailo" } },
    data: {
      message: {
        id: "msg_consent",
        session_id: "session_showcase_approvals",
        sequence: 3,
        role: "user",
        content: [{ type: "text", text: "Yes, create it." }],
        tool_call_id: null,
        created_at: "2026-10-02T22:56:40Z",
      },
    },
  };
  return buildApprovalEpisodes(
    [
      approvalToolEvent(
        "grant-only",
        "record_approval",
        { action: "Commits on this branch need no further ask", detail: "Category exemption." },
        1,
      ),
      approvalToolEvent(
        "ask",
        "request_approval",
        {
          action:
            'Create the reusable organisation-wide agent "SRE Simulator" using the built-in Generic harness and the proposed simulation-only instructions',
          question:
            "Create the SRE Simulator agent with the proposed safe, simulation-only configuration?",
        },
        2,
      ),
      consent,
      approvalToolEvent(
        "grant",
        "record_approval",
        {
          action:
            "Create the organisation-wide SRE Simulator agent using the built-in Generic harness and simulation-only instructions",
          detail:
            "Approved configuration: safety-first, simulation-only SRE training and analysis agent with no external integrations.",
          approved_in_message: "msg_consent",
        },
        4,
      ),
      approvalToolEvent(
        "open-ask",
        "request_approval",
        {
          action: "Publish the SRE Simulator harness to the organisation",
          question: "Publish this harness for every team?",
        },
        5,
      ),
    ],
    {
      memberNames: new Map([["user_mykhailo", "Mykhailo Chalyi"]]),
      inputsComplete: true,
    },
  );
}
