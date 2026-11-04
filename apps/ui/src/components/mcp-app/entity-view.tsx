"use client";
import { useQuery } from "@tanstack/react-query";
import Link from "next/link";
import { AgentCard } from "@/components/agents/agent-card";
import { HarnessCard } from "@/components/harnesses/harness-card";
import { CapabilityCard } from "@/components/capabilities/capability-card";
import { KnowledgeCard } from "@/components/knowledge/knowledge-card";
import { RunBars } from "@/components/agents/home/run-bars";
import { Skeleton } from "@/components/ui/skeleton";
import { useAgent, useHarness, useHarnesses, useCapabilities, useCapability } from "@/hooks";
import { useAgentActivity } from "@/hooks/use-agent-activity";
import { resolveHarnessInheritance } from "@/lib/harness-inheritance";
import { getKnowledge } from "@/lib/api/knowledge";
import { useUiTranslations, useLocale } from "@/providers/locale-provider";
import { formatCountLabel } from "@/lib/formatting";

function Result<T>({
  query,
  children,
}: {
  query: { data?: T; error: Error | null };
  children: (data: T) => React.ReactNode;
}) {
  if (query.error)
    return (
      <p role="alert" className="text-sm text-destructive">
        {query.error.message}
      </p>
    );
  if (!query.data) return <Skeleton className="h-48 w-full" />;
  return children(query.data);
}

export function AgentView({ id }: { id: string }) {
  const agent = useAgent(id);
  const capabilities = useCapabilities();
  const activity = useAgentActivity();
  const item = activity.data?.agents.find((agent) => agent.agent_id === id);
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  return (
    <Result query={agent}>
      {(agent) => (
        <>
          <AgentCard agent={agent} allCapabilities={capabilities.data} />
          {item && (
            <div className="border border-t-0 p-4">
              <div className="mb-3 flex justify-between text-xs text-muted-foreground">
                <span>{ui("Last 24 hours")}</span>
                <Link href={`/agents/${id}/sessions`} className="hover:underline">
                  {formatCountLabel(item.runs, "run", undefined, displayLocale)}
                </Link>
              </div>
              <RunBars buckets={item.hourly} label={ui("Last 24 hours")} />
            </div>
          )}
        </>
      )}
    </Result>
  );
}
export function HarnessView({ id }: { id: string }) {
  const harness = useHarness(id);
  const harnesses = useHarnesses({ includeArchived: true });
  const capabilities = useCapabilities();
  return (
    <Result query={harness}>
      {(harness) => (
        <HarnessCard
          harness={harness}
          allCapabilities={capabilities.data}
          inheritance={resolveHarnessInheritance(
            harness,
            new Map((harnesses.data ?? []).map((item) => [item.id, item])),
          )}
        />
      )}
    </Result>
  );
}
export function CapabilityView({ id }: { id: string }) {
  const capability = useCapability(id);
  return (
    <Result query={capability}>{(capability) => <CapabilityCard capability={capability} />}</Result>
  );
}
export function KnowledgeView({ id }: { id: string }) {
  const collection = useQuery({ queryKey: ["mcp-knowledge", id], queryFn: () => getKnowledge(id) });
  return (
    <Result query={collection}>{(collection) => <KnowledgeCard collection={collection} />}</Result>
  );
}
