"use client";

import { use, useState, useCallback } from "react";
import {
  useEval,
  useEvalCases,
  useEvalRuns,
  useCreateEvalCase,
  useDeleteEvalCase,
  useCreateEvalRun,
} from "@/hooks";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import { CopyButton } from "@/components/ui/copy-button";
import { ArrowLeft, Pencil, Play, Plus, Trash2, ListChecks, BarChart3 } from "lucide-react";
import { getEntityNameClassName, getEntityStatusBadgeVariant } from "@/lib/entity-lifecycle";
import type { EvalCase, EvalRun, Scorer, EvalInputMessage, EvalTarget } from "@/lib/api/types";

function passRateColor(rate: number): string {
  if (rate >= 0.9) return "text-green-600";
  if (rate >= 0.7) return "text-yellow-600";
  return "text-red-600";
}

function runStatusVariant(status: string): "default" | "secondary" | "destructive" | "outline" {
  switch (status) {
    case "completed":
      return "default";
    case "running":
    case "pending":
      return "secondary";
    case "failed":
    case "cancelled":
      return "destructive";
    default:
      return "outline";
  }
}

function scorerLabel(scorer: Scorer): string {
  switch (scorer.type) {
    case "contains":
      return `contains("${scorer.text}")`;
    case "not_contains":
      return `not_contains("${scorer.text}")`;
    case "regex":
      return `regex(/${scorer.pattern}/)`;
    case "tool_called":
      return `tool_called("${scorer.tool}")`;
    case "tool_not_called":
      return `tool_not_called("${scorer.tool}")`;
    case "tool_call_count":
      return `tool_call_count(${scorer.min ?? 0}-${scorer.max ?? "inf"})`;
    case "turns_within":
      return `turns_within(${scorer.max})`;
    case "file_contains":
      return `file_contains("${scorer.path}", "${scorer.text}")`;
    case "json_schema":
      return "json_schema(...)";
    default:
      return "unknown";
  }
}

function TargetBadge({ target }: { target?: EvalTarget }) {
  if (!target) return null;
  switch (target.type) {
    case "harness":
      return (
        <Badge variant="outline" className="text-xs">
          harness
        </Badge>
      );
    case "session_params":
      return (
        <Badge variant="outline" className="text-xs">
          session_params
        </Badge>
      );
    case "app":
      return (
        <Badge variant="outline" className="text-xs">
          app: {target.app_id}
        </Badge>
      );
    default:
      return null;
  }
}

function CaseCard({
  evalCase,
  onDelete,
  deleting,
}: {
  evalCase: EvalCase;
  onDelete: (id: string) => void;
  deleting: boolean;
}) {
  return (
    <Card>
      <CardHeader className="pb-2">
        <div className="flex items-center justify-between">
          <CardTitle className="text-base">{evalCase.name}</CardTitle>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => onDelete(evalCase.id)}
            disabled={deleting}
          >
            <Trash2 className="w-4 h-4 text-muted-foreground" />
          </Button>
        </div>
      </CardHeader>
      <CardContent className="space-y-3">
        {evalCase.description && (
          <p className="text-sm text-muted-foreground">{evalCase.description}</p>
        )}
        {evalCase.target && (
          <div className="flex items-center gap-2">
            <span className="text-xs text-muted-foreground">Target override:</span>
            <TargetBadge target={evalCase.target} />
          </div>
        )}
        <div className="space-y-1">
          <p className="text-xs font-medium text-muted-foreground">Messages</p>
          {evalCase.conversation.map((msg, i) => (
            <div key={i} className="text-sm bg-muted/50 rounded px-2 py-1">
              {msg.content}
            </div>
          ))}
        </div>
        <div className="space-y-1">
          <p className="text-xs font-medium text-muted-foreground">Scorers</p>
          <div className="flex flex-wrap gap-1">
            {evalCase.scorers.map((scorer, i) => (
              <Badge key={i} variant="outline" className="text-xs font-mono">
                {scorerLabel(scorer)}
              </Badge>
            ))}
          </div>
        </div>
        {(evalCase.max_turns || evalCase.timeout_seconds) && (
          <div className="flex gap-3 text-xs text-muted-foreground">
            {evalCase.max_turns && <span>Max turns: {evalCase.max_turns}</span>}
            {evalCase.timeout_seconds && <span>Timeout: {evalCase.timeout_seconds}s</span>}
          </div>
        )}
        {evalCase.tags.length > 0 && (
          <div className="flex flex-wrap gap-1">
            {evalCase.tags.map((tag) => (
              <Badge key={tag} variant="secondary" className="text-xs">
                {tag}
              </Badge>
            ))}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function RunRow({ evalId, run }: { evalId: string; run: EvalRun }) {
  return (
    <Link href={`/evals/${evalId}/runs/${run.id}`} className="block">
      <Card className="hover:border-primary/50 transition-colors cursor-pointer">
        <CardContent className="flex items-center justify-between py-4">
          <div className="flex items-center gap-4">
            <Badge variant={runStatusVariant(run.status)}>{run.status}</Badge>
            <span className="text-sm text-muted-foreground">
              {new Date(run.created_at).toLocaleString()}
            </span>
            <span className="text-xs text-muted-foreground">by {run.triggered_by}</span>
            {run.target && <TargetBadge target={run.target} />}
          </div>
          <div className="flex items-center gap-4 text-sm">
            {run.summary && (
              <>
                <span className={passRateColor(run.summary.pass_rate)}>
                  {(run.summary.pass_rate * 100).toFixed(0)}% pass
                </span>
                <span className="text-muted-foreground">
                  {run.summary.passed}/{run.summary.total} passed
                </span>
              </>
            )}
          </div>
        </CardContent>
      </Card>
    </Link>
  );
}

function AddCaseForm({ evalId, onCreated }: { evalId: string; onCreated: () => void }) {
  const createCase = useCreateEvalCase(evalId);
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [messageContent, setMessageContent] = useState("");
  const [scorerType, setScorerType] = useState<
    "contains" | "not_contains" | "regex" | "tool_called"
  >("contains");
  const [scorerValue, setScorerValue] = useState("");

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();

    const conversation: EvalInputMessage[] = messageContent
      .split("\n")
      .filter((line) => line.trim())
      .map((content) => ({ content: content.trim() }));

    const scorers: Scorer[] = scorerValue
      ? scorerValue.split(",").map((val) => {
          const v = val.trim();
          switch (scorerType) {
            case "contains":
              return { type: "contains" as const, text: v };
            case "not_contains":
              return { type: "not_contains" as const, text: v };
            case "regex":
              return { type: "regex" as const, pattern: v };
            case "tool_called":
              return { type: "tool_called" as const, tool: v };
          }
        })
      : [];

    try {
      await createCase.mutateAsync({
        name,
        description: description || undefined,
        conversation,
        scorers,
      });
      setName("");
      setDescription("");
      setMessageContent("");
      setScorerValue("");
      onCreated();
    } catch (error) {
      console.error("Failed to create case:", error);
    }
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">Add Test Case</CardTitle>
      </CardHeader>
      <CardContent>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="case-name">Name</Label>
            <Input
              id="case-name"
              placeholder="Test case name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              required
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="case-description">Description (optional)</Label>
            <Input
              id="case-description"
              placeholder="What this test verifies"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="case-messages">Messages (one per line)</Label>
            <Textarea
              id="case-messages"
              placeholder="Hello, can you help me write a Python script?"
              value={messageContent}
              onChange={(e) => setMessageContent(e.target.value)}
              rows={3}
              required
            />
          </div>
          <div className="space-y-2">
            <Label>Scorers</Label>
            <div className="flex gap-2">
              <select
                value={scorerType}
                onChange={(e) => setScorerType(e.target.value as typeof scorerType)}
                className="border rounded px-2 py-1 text-sm bg-background"
              >
                <option value="contains">contains</option>
                <option value="not_contains">not_contains</option>
                <option value="regex">regex</option>
                <option value="tool_called">tool_called</option>
              </select>
              <Input
                placeholder="Value (comma-separated for multiple)"
                value={scorerValue}
                onChange={(e) => setScorerValue(e.target.value)}
              />
            </div>
          </div>
          <Button type="submit" size="sm" disabled={createCase.isPending}>
            {createCase.isPending ? "Adding..." : "Add Case"}
          </Button>
          {createCase.error && (
            <p className="text-sm text-destructive">Error: {createCase.error.message}</p>
          )}
        </form>
      </CardContent>
    </Card>
  );
}

export default function EvalDetailPage({ params }: { params: Promise<{ evalId: string }> }) {
  const { evalId } = use(params);
  const router = useRouter();
  const [activeTab, setActiveTab] = useState("cases");
  const [showAddCase, setShowAddCase] = useState(false);

  const { data: ev, isLoading: evalLoading } = useEval(evalId);
  const { data: cases, isLoading: casesLoading } = useEvalCases(evalId);
  const { data: runs, isLoading: runsLoading } = useEvalRuns(evalId);
  const deleteCase = useDeleteEvalCase(evalId);
  const createRun = useCreateEvalRun(evalId);

  const handleDeleteCase = useCallback(
    async (caseId: string) => {
      try {
        await deleteCase.mutateAsync(caseId);
      } catch (error) {
        console.error("Failed to delete case:", error);
      }
    },
    [deleteCase],
  );

  const handleStartRun = useCallback(async () => {
    try {
      const run = await createRun.mutateAsync({});
      router.push(`/evals/${evalId}/runs/${run.id}`);
    } catch (error) {
      console.error("Failed to start run:", error);
    }
  }, [createRun, evalId, router]);

  if (evalLoading) {
    return (
      <div className="container mx-auto p-6">
        <Skeleton className="h-8 w-1/3 mb-4" />
        <Skeleton className="h-4 w-2/3 mb-8" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  if (!ev) {
    return (
      <div className="container mx-auto p-6">
        <div className="text-red-500">Eval not found</div>
        <Link href="/evals" className="text-blue-500 hover:underline">
          Back to evals
        </Link>
      </div>
    );
  }

  return (
    <div className="container mx-auto p-6">
      <Link
        href="/evals"
        className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-6"
      >
        <ArrowLeft className="w-4 h-4 mr-2" />
        Back to Evals
      </Link>

      <div className="flex items-start justify-between mb-6">
        <div>
          <h1 className="text-2xl font-bold flex items-center gap-2">
            <span className={getEntityNameClassName(ev.status)}>{ev.name}</span>
            <CopyButton value={ev.id} />
            <Badge variant={getEntityStatusBadgeVariant(ev.status)}>{ev.status}</Badge>
          </h1>
          {ev.description && <p className="text-muted-foreground mt-1">{ev.description}</p>}
          <div className="flex items-center gap-4 mt-2 text-sm text-muted-foreground">
            <TargetBadge target={ev.target} />
            <span>{ev.case_count} cases</span>
            {ev.tags.length > 0 && (
              <div className="flex gap-1">
                {ev.tags.map((tag) => (
                  <Badge key={tag} variant="outline" className="text-xs">
                    {tag}
                  </Badge>
                ))}
              </div>
            )}
          </div>
          {ev.last_run?.summary && (
            <div className="flex items-center gap-3 mt-2 text-sm">
              <span>Last run:</span>
              <Badge variant={runStatusVariant(ev.last_run.status)}>{ev.last_run.status}</Badge>
              <span className={passRateColor(ev.last_run.summary.pass_rate)}>
                {(ev.last_run.summary.pass_rate * 100).toFixed(0)}% pass rate
              </span>
            </div>
          )}
        </div>
        <div className="flex gap-2">
          <Link href={`/evals/${evalId}/edit`}>
            <Button variant="outline">
              <Pencil className="w-4 h-4 mr-2" />
              Edit
            </Button>
          </Link>
          <Button
            variant="accent"
            onClick={handleStartRun}
            disabled={createRun.isPending || ev.status !== "active"}
          >
            <Play className="w-4 h-4 mr-2" />
            {createRun.isPending ? "Starting..." : "Run Eval"}
          </Button>
        </div>
      </div>

      <Tabs value={activeTab} onValueChange={setActiveTab}>
        <TabsList className="mb-6">
          <TabsTrigger value="cases">
            <ListChecks className="w-4 h-4 mr-2" />
            Cases ({cases?.length ?? 0})
          </TabsTrigger>
          <TabsTrigger value="runs">
            <BarChart3 className="w-4 h-4 mr-2" />
            Runs ({runs?.length ?? 0})
          </TabsTrigger>
        </TabsList>

        <TabsContent value="cases">
          <div className="space-y-4">
            <div className="flex justify-end">
              <Button variant="outline" size="sm" onClick={() => setShowAddCase(!showAddCase)}>
                <Plus className="w-4 h-4 mr-2" />
                {showAddCase ? "Cancel" : "Add Case"}
              </Button>
            </div>

            {showAddCase && <AddCaseForm evalId={evalId} onCreated={() => setShowAddCase(false)} />}

            {casesLoading ? (
              <div className="space-y-3">
                <Skeleton className="h-32 w-full" />
                <Skeleton className="h-32 w-full" />
              </div>
            ) : !cases || cases.length === 0 ? (
              <div className="text-center py-12">
                <p className="text-muted-foreground mb-4">No test cases yet</p>
                <Button onClick={() => setShowAddCase(true)}>
                  <Plus className="w-4 h-4 mr-2" />
                  Add your first test case
                </Button>
              </div>
            ) : (
              <div className="grid gap-4 md:grid-cols-2">
                {cases.map((c) => (
                  <CaseCard
                    key={c.id}
                    evalCase={c}
                    onDelete={handleDeleteCase}
                    deleting={deleteCase.isPending}
                  />
                ))}
              </div>
            )}
          </div>
        </TabsContent>

        <TabsContent value="runs">
          <div className="space-y-4">
            {runsLoading ? (
              <div className="space-y-3">
                <Skeleton className="h-16 w-full" />
                <Skeleton className="h-16 w-full" />
              </div>
            ) : !runs || runs.length === 0 ? (
              <div className="text-center py-12">
                <p className="text-muted-foreground mb-4">No runs yet</p>
                <Button onClick={handleStartRun} disabled={createRun.isPending}>
                  <Play className="w-4 h-4 mr-2" />
                  Start your first run
                </Button>
              </div>
            ) : (
              <div className="space-y-3">
                {runs.map((run) => (
                  <RunRow key={run.id} evalId={evalId} run={run} />
                ))}
              </div>
            )}
          </div>
        </TabsContent>
      </Tabs>
    </div>
  );
}
