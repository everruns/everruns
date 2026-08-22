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
import { SessionCard } from "@/components/session/session-card";

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
