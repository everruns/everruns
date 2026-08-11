"use client";

import { use, useState } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import {
  useObserver,
  useObserverScores,
  useUpdateObserver,
  useDeleteObserver,
  useAgents,
  useHarnesses,
  usePageTitle,
} from "@/hooks";
import { ResourceNotFound } from "@/components/resource-not-found";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { EntityIdentity } from "@/components/ui/entity-identity";
import { Notice, NoticeDescription } from "@/components/ui/notice";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Pencil, Pause, Play, Archive, Settings, BarChart3 } from "lucide-react";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { describeRule } from "@/components/observers/scorer-editor";
import { QualitySummary } from "@/components/observers/quality-summary";
import {
  getEntityNameClassName,
  getEntityStatusBadgeVariant,
  getDisplayName,
} from "@/lib/entity-lifecycle";
import type { Observer, ObserverScorerConfig, TraceScore } from "@/lib/api/types";

function scoreStatusVariant(status: string): "default" | "secondary" | "destructive" | "outline" {
  switch (status) {
    case "completed":
      return "default";
    case "pending":
    case "scoring":
      return "secondary";
    case "errored":
      return "destructive";
    default:
      return "outline";
  }
}

function scorerSummary(scorer: ObserverScorerConfig): string {
  if (scorer.method === "llm_judge") {
    return `llm_judge (pass ≥ ${scorer.pass_threshold})`;
  }
  return describeRule(scorer.rule);
}

function ConfigurationTab({
  observer,
  agentNames,
  harnessNames,
}: {
  observer: Observer;
  agentNames: Map<string, string>;
  harnessNames: Map<string, string>;
}) {
  const { agent_ids, harness_ids, session_tags } = observer.match;
  const hasMatch =
    (agent_ids?.length ?? 0) > 0 ||
    (harness_ids?.length ?? 0) > 0 ||
    (session_tags?.length ?? 0) > 0;

  return (
    <div className="grid gap-4 md:grid-cols-2">
      <Card>
        <CardHeader>
          <CardTitle className="text-base">Match rules</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3 text-sm">
          {!hasMatch && (
            <p className="text-muted-foreground">No filters — considers all production sessions.</p>
          )}
          {agent_ids && agent_ids.length > 0 && (
            <div>
              <p className="mb-1 text-xs font-medium text-muted-foreground">Agents</p>
              <div className="flex flex-wrap gap-1">
                {agent_ids.map((id) => (
                  <Badge key={id} variant="outline">
                    {agentNames.get(id) ?? id}
                  </Badge>
                ))}
              </div>
            </div>
          )}
          {harness_ids && harness_ids.length > 0 && (
            <div>
              <p className="mb-1 text-xs font-medium text-muted-foreground">Harnesses</p>
              <div className="flex flex-wrap gap-1">
                {harness_ids.map((id) => (
                  <Badge key={id} variant="outline">
                    {harnessNames.get(id) ?? id}
                  </Badge>
                ))}
              </div>
            </div>
          )}
          {session_tags && session_tags.length > 0 && (
            <div>
              <p className="mb-1 text-xs font-medium text-muted-foreground">Session tags</p>
              <div className="flex flex-wrap gap-1">
                {session_tags.map((tag) => (
                  <Badge key={tag} variant="secondary">
                    {tag}
                  </Badge>
                ))}
              </div>
            </div>
          )}
          <div>
            <p className="mb-1 text-xs font-medium text-muted-foreground">Sampling rate</p>
            <p>{Math.round(observer.sampling_rate * 100)}% of matching turns</p>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-base">Scorers</CardTitle>
        </CardHeader>
        <CardContent className="space-y-2">
          {observer.scorers.length === 0 ? (
            <p className="text-sm text-muted-foreground">No scorers configured.</p>
          ) : (
            observer.scorers.map((scorer) => (
              <div
                key={scorer.key}
                className="flex items-center justify-between gap-2 border bg-muted/50 p-2"
              >
                <span className="font-mono text-sm">{scorer.key}</span>
                <span className="text-xs text-muted-foreground">{scorerSummary(scorer)}</span>
              </div>
            ))
          )}
        </CardContent>
      </Card>
    </div>
  );
}

function ScoreRow({ score }: { score: TraceScore }) {
  const reason = score.reason ?? score.error_message ?? "";
  const truncated = reason.length > 80 ? `${reason.slice(0, 80)}…` : reason;
  return (
    <TableRow>
      <TableCell className="font-mono text-xs">{score.scorer_key}</TableCell>
      <TableCell>{score.value !== undefined ? score.value.toFixed(2) : "—"}</TableCell>
      <TableCell>
        {score.pass === undefined ? (
          "—"
        ) : (
          <Badge variant={score.pass ? "default" : "destructive"}>
            {score.pass ? "pass" : "fail"}
          </Badge>
        )}
      </TableCell>
      <TableCell>{score.label ?? "—"}</TableCell>
      <TableCell>
        <Badge variant={scoreStatusVariant(score.status)}>{score.status}</Badge>
      </TableCell>
      <TableCell className="max-w-[240px] text-xs text-muted-foreground">
        {truncated || "—"}
      </TableCell>
      <TableCell className="text-xs text-muted-foreground">
        {new Date(score.created_at).toLocaleString()}
      </TableCell>
      <TableCell>
        <Link
          href={`/sessions/${score.session_id}/transcript`}
          className="text-primary hover:underline"
        >
          View
        </Link>
      </TableCell>
    </TableRow>
  );
}

function QualityTab({ observerId }: { observerId: string }) {
  const { data: scores, isLoading, error } = useObserverScores(observerId);

  return (
    <div className="space-y-6">
      <Notice variant="info">
        <NoticeDescription>
          Trends are computed from recent sampled scores; full aggregation is a follow-up.
        </NoticeDescription>
      </Notice>

      <QueryStateWrapper
        isLoading={isLoading}
        error={error}
        data={scores}
        errorMessagePrefix="Failed to load scores"
        emptyState={
          <div className="py-12 text-center text-muted-foreground">
            No scores yet. Scores appear here once matching production turns are sampled.
          </div>
        }
      >
        {(items) => (
          <>
            <QualitySummary scores={items} />

            <Card>
              <CardHeader>
                <CardTitle className="text-base">Recent scores</CardTitle>
              </CardHeader>
              <CardContent>
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Scorer</TableHead>
                      <TableHead>Value</TableHead>
                      <TableHead>Pass</TableHead>
                      <TableHead>Label</TableHead>
                      <TableHead>Status</TableHead>
                      <TableHead>Reasoning</TableHead>
                      <TableHead>Created</TableHead>
                      <TableHead>Session</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {items.map((score) => (
                      <ScoreRow key={score.id} score={score} />
                    ))}
                  </TableBody>
                </Table>
              </CardContent>
            </Card>
          </>
        )}
      </QueryStateWrapper>
    </div>
  );
}

export default function ObserverDetailPage({
  params,
}: {
  params: Promise<{ observerId: string }>;
}) {
  const { observerId } = use(params);
  const router = useRouter();
  const [activeTab, setActiveTab] = useState("configuration");

  const { data: observer, isLoading } = useObserver(observerId);
  usePageTitle(observer?.name ?? null, "Observer");
  const { data: agents = [] } = useAgents();
  const { data: harnesses = [] } = useHarnesses();
  const updateObserver = useUpdateObserver();
  const deleteObserver = useDeleteObserver();

  const agentNames = new Map(agents.map((a) => [a.id, getDisplayName(a)]));
  const harnessNames = new Map(harnesses.map((h) => [h.id, getDisplayName(h)]));

  const handleToggleStatus = async () => {
    if (!observer) return;
    const nextStatus = observer.status === "active" ? "paused" : "active";
    try {
      await updateObserver.mutateAsync({
        observerId,
        request: { status: nextStatus },
      });
    } catch (error) {
      console.error("Failed to update observer status:", error);
    }
  };

  const handleArchive = async () => {
    try {
      await deleteObserver.mutateAsync(observerId);
      router.push("/observers");
    } catch (error) {
      console.error("Failed to archive observer:", error);
    }
  };

  if (isLoading) {
    return (
      <div className="container mx-auto p-6">
        <Skeleton className="mb-4 h-8 w-1/3" />
        <Skeleton className="mb-8 h-4 w-2/3" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  if (!observer) {
    return (
      <ResourceNotFound
        title="Observer not found"
        description="This observer may have been deleted, moved to another organization, or the URL may be wrong."
        backHref="/observers"
        backLabel="Back to observers"
        resourceId={observerId}
      />
    );
  }

  const isArchived = observer.status === "archived";

  return (
    <div className="container mx-auto p-6">
      <Link
        href="/observers"
        className="mb-6 inline-flex items-center text-sm text-muted-foreground hover:text-foreground"
      >
        Back to Observers
      </Link>

      <div className="mb-6 flex items-start justify-between">
        <div>
          <h1 className="flex min-w-0 flex-wrap items-center gap-2 text-2xl font-bold">
            <EntityIdentity
              value={observer.id}
              labelClassName={getEntityNameClassName(observer.status)}
            >
              {observer.name}
            </EntityIdentity>
            <Badge variant={getEntityStatusBadgeVariant(observer.status)}>{observer.status}</Badge>
          </h1>
          {observer.description && (
            <p className="mt-1 text-muted-foreground">{observer.description}</p>
          )}
        </div>
        <div className="flex gap-2">
          {!isArchived && (
            <Link href={`/observers/${observerId}/edit`}>
              <Button variant="outline">
                <Pencil className="mr-2 h-4 w-4" />
                Edit
              </Button>
            </Link>
          )}
          {!isArchived && (
            <Button
              variant="outline"
              onClick={handleToggleStatus}
              disabled={updateObserver.isPending}
            >
              {observer.status === "active" ? (
                <>
                  <Pause className="mr-2 h-4 w-4" />
                  Pause
                </>
              ) : (
                <>
                  <Play className="mr-2 h-4 w-4" />
                  Resume
                </>
              )}
            </Button>
          )}
          {!isArchived && (
            <Button variant="outline" onClick={handleArchive} disabled={deleteObserver.isPending}>
              <Archive className="mr-2 h-4 w-4" />
              Archive
            </Button>
          )}
        </div>
      </div>

      <Tabs value={activeTab} onValueChange={setActiveTab}>
        <TabsList className="mb-6">
          <TabsTrigger value="configuration">
            <Settings className="mr-2 h-4 w-4" />
            Configuration
          </TabsTrigger>
          <TabsTrigger value="quality">
            <BarChart3 className="mr-2 h-4 w-4" />
            Quality
          </TabsTrigger>
        </TabsList>

        <TabsContent value="configuration">
          <ConfigurationTab
            observer={observer}
            agentNames={agentNames}
            harnessNames={harnessNames}
          />
        </TabsContent>

        <TabsContent value="quality">
          <QualityTab observerId={observerId} />
        </TabsContent>
      </Tabs>
    </div>
  );
}
