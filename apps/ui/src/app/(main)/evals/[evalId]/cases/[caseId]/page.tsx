"use client";

// One eval case (approved mockup 05): a scripted or simulated conversation,
// the case's setup and tool fixtures, its graders, and its recent results.
// "Try once" runs only this case and opens the run.

import { use, useEffect, useMemo, useState } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { Play, Plus, Save, Trash2 } from "lucide-react";
import { useUiTranslations } from "@/providers/locale-provider";
import {
  useCreateEvalRun,
  useEval,
  useEvalCases,
  useEvalRunComparison,
  useEvalRuns,
  useModels,
  usePageTitle,
  useUpdateEvalCase,
} from "@/hooks";
import { useVirtualUsers } from "@/hooks/use-virtual-users";
import { usePolicies } from "@/hooks/use-policies";
import { EvalsIcon } from "@/components/icons/facet-icons";
import { ResourceNotFound } from "@/components/resource-not-found";
import { PageBreadcrumb, PageContainer, PageMain, PageMasthead } from "@/components/layout";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Field } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Notice } from "@/components/ui/notice";
import { SegmentedControl } from "@/components/ui/segmented-control";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import {
  type CaseDraft,
  type DraftError,
  MAX_SIMULATED_TURNS,
  caseMode,
  draftFromCase,
  scorerSummary,
  simulationOutcome,
  updateFromDraft,
} from "@/components/evals/case-format";
import type { EvalCase, EvalCaseResult, Scorer } from "@/lib/api/types";

const RECENT_RUNS = 5;
const ORG_DEFAULT = "__default";
const ANONYMOUS = "__anonymous";

export default function EvalCasePage({
  params,
}: {
  params: Promise<{ evalId: string; caseId: string }>;
}) {
  const { evalId, caseId } = use(params);
  const ui = useUiTranslations();
  const { data: ev } = useEval(evalId);
  const { data: cases, isLoading } = useEvalCases(evalId);
  const evalCase = cases?.find((c) => c.id === caseId);
  usePageTitle(evalCase?.name ?? ui("Case"), ev?.name ?? null, ui("Evals"));

  if (!isLoading && cases && !evalCase) {
    return (
      <ResourceNotFound
        title={ui("Case not found")}
        description={ui(
          "This case may have been deleted, moved to another organization, or the URL may be wrong.",
        )}
        backHref={`/evals/${evalId}`}
        backLabel="Back to dataset"
        resourceId={caseId}
      />
    );
  }

  return (
    <PageContainer>
      <PageBreadcrumb
        items={[
          { label: ui("Evals"), href: "/evals" },
          { label: ui("Datasets"), href: "/evals?tab=datasets" },
          { label: ev?.name ?? ui("Dataset"), href: `/evals/${evalId}` },
          { label: evalCase?.name ?? ui("Case") },
        ]}
      />
      {!evalCase || !ev ? (
        <PageMain>
          <Skeleton className="h-96 w-full" />
        </PageMain>
      ) : (
        <CaseEditor
          key={evalCase.updated_at}
          evalId={evalId}
          evalName={ev.name}
          version={ev.version}
          evalCase={evalCase}
        />
      )}
    </PageContainer>
  );
}

function CaseEditor({
  evalId,
  evalName,
  version,
  evalCase,
}: {
  evalId: string;
  evalName: string;
  version?: number;
  evalCase: EvalCase;
}) {
  const ui = useUiTranslations();
  const router = useRouter();
  const { can } = usePolicies("agents");
  const writable = can("agent.manage");
  const [draft, setDraft] = useState<CaseDraft>(() => draftFromCase(evalCase));
  const [tab, setTab] = useState("conversation");
  const [error, setError] = useState<DraftError | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const update = useUpdateEvalCase(evalId, evalCase.id);
  const createRun = useCreateEvalRun(evalId);
  const dirty = useMemo(
    () => JSON.stringify(draft) !== JSON.stringify(draftFromCase(evalCase)),
    [draft, evalCase],
  );

  const set = <K extends keyof CaseDraft>(key: K, value: CaseDraft[K]) =>
    setDraft((d) => ({ ...d, [key]: value }));

  const save = async (): Promise<boolean> => {
    const built = updateFromDraft(draft);
    if ("error" in built) {
      setError(built.error);
      if (built.error.field === "fixture" || built.error.field === "timeoutSeconds") {
        setTab("setup");
      } else {
        setTab("conversation");
      }
      return false;
    }
    setError(null);
    setSaveError(null);
    try {
      await update.mutateAsync(built.request);
      return true;
    } catch (e) {
      setSaveError(e instanceof Error ? e.message : String(e));
      return false;
    }
  };

  const tryOnce = async () => {
    if (dirty && !(await save())) return;
    try {
      const run = await createRun.mutateAsync({ case_ids: [evalCase.id] });
      router.push(`/evals/${evalId}/runs/${run.id}`);
    } catch (e) {
      setSaveError(e instanceof Error ? e.message : String(e));
    }
  };

  const savedMode = caseMode(evalCase);
  const description = [
    savedMode === "simulated" ? ui("Simulated conversation") : ui("Scripted conversation"),
    version !== undefined
      ? ui("{name} v{version} (editing creates v{next})", {
          name: evalName,
          version,
          next: version + 1,
        })
      : evalName,
  ].join(" · ");

  return (
    <>
      <PageMasthead
        icon={<EvalsIcon />}
        title={evalCase.name}
        description={description}
        actions={
          <div className="flex items-center gap-2">
            <Button
              variant="outline"
              onClick={tryOnce}
              disabled={!writable || createRun.isPending || update.isPending}
            >
              <Play className="mr-2 h-4 w-4" />
              {ui("Try once")}
            </Button>
            <Button onClick={save} disabled={!writable || !dirty || update.isPending}>
              <Save className="mr-2 h-4 w-4" />
              {ui("Save")}
            </Button>
          </div>
        }
      />
      <PageMain className="space-y-4">
        {saveError && (
          <Notice variant="destructive">
            <p className="text-sm">{saveError}</p>
          </Notice>
        )}
        <Tabs value={tab} onValueChange={setTab}>
          <TabsList className="mb-6">
            <TabsTrigger value="conversation">{ui("Conversation")}</TabsTrigger>
            <TabsTrigger value="setup">{ui("Setup")}</TabsTrigger>
            <TabsTrigger value="graders">{ui("Graders")}</TabsTrigger>
            <TabsTrigger value="runs">{ui("Runs")}</TabsTrigger>
          </TabsList>
          <TabsContent value="conversation">
            <div className="grid gap-6 lg:grid-cols-[minmax(0,1fr)_320px]">
              <ConversationEditor draft={draft} set={set} error={error} writable={writable} />
              <aside className="space-y-4">
                <GradersCard scorers={evalCase.scorers} />
                <FixturesCard draft={draft} onEdit={() => setTab("setup")} />
              </aside>
            </div>
          </TabsContent>
          <TabsContent value="setup">
            <SetupEditor draft={draft} setDraft={setDraft} error={error} writable={writable} />
          </TabsContent>
          <TabsContent value="graders">
            <GradersCard scorers={evalCase.scorers} />
          </TabsContent>
          <TabsContent value="runs">
            <CaseRuns evalId={evalId} caseId={evalCase.id} />
          </TabsContent>
        </Tabs>
      </PageMain>
    </>
  );
}

function ConversationEditor({
  draft,
  set,
  error,
  writable,
}: {
  draft: CaseDraft;
  set: <K extends keyof CaseDraft>(key: K, value: CaseDraft[K]) => void;
  error: DraftError | null;
  writable: boolean;
}) {
  const ui = useUiTranslations();
  const { data: models } = useModels();
  const { data: virtualUsers } = useVirtualUsers({ usage: "end_user" });
  const required = ui("Required.");
  return (
    <div className="space-y-4">
      <SegmentedControl
        label={ui("Conversation")}
        value={draft.mode}
        onValueChange={(value) => set("mode", value as CaseDraft["mode"])}
        options={[
          { value: "scripted", label: ui("Scripted messages") },
          { value: "simulated", label: ui("Simulated user") },
        ]}
      />
      {draft.mode === "simulated" && (
        <>
          <Field
            id="case-persona"
            label={ui("Persona")}
            description={ui("Who the user is, how they write, and what they know.")}
            error={error?.field === "persona" ? required : undefined}
          >
            <Textarea
              id="case-persona"
              rows={3}
              value={draft.persona}
              disabled={!writable}
              onChange={(e) => set("persona", e.target.value)}
            />
          </Field>
          <Field
            id="case-goal"
            label={ui("Goal")}
            error={error?.field === "goal" ? required : undefined}
          >
            <Textarea
              id="case-goal"
              rows={2}
              value={draft.goal}
              disabled={!writable}
              onChange={(e) => set("goal", e.target.value)}
            />
          </Field>
          <Field id="case-done" label={ui("The user is done when")}>
            <Textarea
              id="case-done"
              rows={2}
              value={draft.doneWhen}
              disabled={!writable}
              onChange={(e) => set("doneWhen", e.target.value)}
            />
          </Field>
          <div className="grid gap-4 sm:grid-cols-3">
            <Field id="case-virtual-user" label={ui("Plays the user as")}>
              <Select
                value={draft.virtualUserId || ANONYMOUS}
                onValueChange={(v) => set("virtualUserId", v === ANONYMOUS ? "" : v)}
                disabled={!writable}
              >
                <SelectTrigger id="case-virtual-user">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={ANONYMOUS}>{ui("No virtual user")}</SelectItem>
                  {(virtualUsers ?? []).map((user) => (
                    <SelectItem key={user.id} value={user.id}>
                      {user.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </Field>
            <Field id="case-sim-model" label={ui("Simulator model")}>
              <Select
                value={draft.modelId || ORG_DEFAULT}
                onValueChange={(v) => set("modelId", v === ORG_DEFAULT ? "" : v)}
                disabled={!writable}
              >
                <SelectTrigger id="case-sim-model">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={ORG_DEFAULT}>{ui("Organization default")}</SelectItem>
                  {(models ?? []).map((model) => (
                    <SelectItem key={model.id} value={model.id}>
                      {model.display_name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </Field>
            <Field
              id="case-max-turns"
              label={ui("Max turns")}
              error={
                error?.field === "maxTurns"
                  ? ui("Between 1 and {max}.", { max: MAX_SIMULATED_TURNS })
                  : undefined
              }
            >
              <Input
                id="case-max-turns"
                type="number"
                min={1}
                max={MAX_SIMULATED_TURNS}
                value={draft.maxTurns}
                disabled={!writable}
                onChange={(e) => set("maxTurns", e.target.value)}
              />
            </Field>
          </div>
        </>
      )}
      <Field
        id="case-messages"
        label={
          draft.mode === "simulated"
            ? ui("Opening messages (optional, one per line)")
            : ui("Messages (one per line)")
        }
        description={
          draft.mode === "simulated"
            ? ui("Sent first; the simulated user carries on after them.")
            : undefined
        }
        error={
          error?.field === "messages"
            ? ui("A scripted case needs at least one message.")
            : undefined
        }
      >
        <Textarea
          id="case-messages"
          rows={draft.mode === "simulated" ? 2 : 5}
          value={draft.messages}
          disabled={!writable}
          onChange={(e) => set("messages", e.target.value)}
        />
      </Field>
    </div>
  );
}

function SetupEditor({
  draft,
  setDraft,
  error,
  writable,
}: {
  draft: CaseDraft;
  setDraft: React.Dispatch<React.SetStateAction<CaseDraft>>;
  error: DraftError | null;
  writable: boolean;
}) {
  const ui = useUiTranslations();
  const setFixture = (index: number, patch: Partial<CaseDraft["fixtures"][number]>) =>
    setDraft((d) => ({
      ...d,
      fixtures: d.fixtures.map((f, i) => (i === index ? { ...f, ...patch } : f)),
    }));
  return (
    <div className="max-w-3xl space-y-6">
      <Field
        id="case-timeout"
        label={ui("Case timeout (seconds)")}
        description={ui("The whole conversation must finish within it.")}
        error={error?.field === "timeoutSeconds" ? ui("A whole number of seconds.") : undefined}
        className="max-w-xs"
      >
        <Input
          id="case-timeout"
          type="number"
          min={1}
          value={draft.timeoutSeconds}
          disabled={!writable}
          onChange={(e) => setDraft((d) => ({ ...d, timeoutSeconds: e.target.value }))}
        />
      </Field>
      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <div>
            <h3 className="text-sm font-medium">{ui("Tool fixtures")}</h3>
            <p className="text-xs text-muted-foreground">
              {ui(
                "Tools the agent gets in this case. Each call is answered with the canned response and kept in the trace; nothing reaches a real system.",
              )}
            </p>
          </div>
          <Button
            variant="outline"
            size="sm"
            disabled={!writable}
            onClick={() =>
              setDraft((d) => ({
                ...d,
                fixtures: [...d.fixtures, { name: "", description: "", response: "{}" }],
              }))
            }
          >
            <Plus className="mr-2 h-4 w-4" />
            {ui("Add fixture")}
          </Button>
        </div>
        {draft.fixtures.map((fixture, index) => {
          const fixtureError =
            error?.field === "fixture" && error.index === index ? error.reason : null;
          return (
            <Card key={index}>
              <CardContent className="space-y-3 pt-4">
                <div className="grid gap-3 sm:grid-cols-[1fr_2fr_auto] sm:items-end">
                  <Field
                    id={`fixture-name-${index}`}
                    label={ui("Tool name")}
                    error={
                      fixtureError === "name"
                        ? ui("Letters, digits, '_' or '-', up to 64.")
                        : undefined
                    }
                  >
                    <Input
                      id={`fixture-name-${index}`}
                      className="font-mono"
                      value={fixture.name}
                      disabled={!writable}
                      onChange={(e) => setFixture(index, { name: e.target.value })}
                    />
                  </Field>
                  <Field id={`fixture-description-${index}`} label={ui("What it does")}>
                    <Input
                      id={`fixture-description-${index}`}
                      value={fixture.description}
                      disabled={!writable}
                      onChange={(e) => setFixture(index, { description: e.target.value })}
                    />
                  </Field>
                  <Button
                    variant="ghost"
                    size="icon"
                    aria-label={ui("Remove fixture")}
                    disabled={!writable}
                    onClick={() =>
                      setDraft((d) => ({
                        ...d,
                        fixtures: d.fixtures.filter((_, i) => i !== index),
                      }))
                    }
                  >
                    <Trash2 className="h-4 w-4" />
                  </Button>
                </div>
                <Field
                  id={`fixture-response-${index}`}
                  label={ui("Response (JSON)")}
                  error={fixtureError === "response" ? ui("Not valid JSON.") : undefined}
                >
                  <Textarea
                    id={`fixture-response-${index}`}
                    className="font-mono text-xs"
                    rows={4}
                    value={fixture.response}
                    disabled={!writable}
                    onChange={(e) => setFixture(index, { response: e.target.value })}
                  />
                </Field>
              </CardContent>
            </Card>
          );
        })}
      </div>
    </div>
  );
}

function kindLabel(kind: ReturnType<typeof scorerSummary>["kind"]) {
  switch (kind) {
    case "judge":
      return "LLM judge";
    case "decision":
      return "Decision model";
    case "metric":
      return "Metric";
    default:
      return "Rule";
  }
}

function GradersCard({ scorers }: { scorers: Scorer[] }) {
  const ui = useUiTranslations();
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-sm">{ui("Graders")}</CardTitle>
      </CardHeader>
      <CardContent className="divide-y p-0">
        {scorers.length === 0 ? (
          <p className="px-6 py-4 text-sm text-muted-foreground">{ui("No graders yet.")}</p>
        ) : (
          scorers.map((scorer, index) => {
            const summary = scorerSummary(scorer);
            const weight =
              scorer.weight !== undefined && scorer.weight !== 1 ? scorer.weight : null;
            return (
              <div key={index} className="px-6 py-3">
                <p className="text-sm font-medium">
                  {summary.template === "{text}"
                    ? summary.params.text
                    : ui(summary.template, summary.params)}
                </p>
                <p className="text-xs text-muted-foreground">
                  {[
                    ui(kindLabel(summary.kind)),
                    summary.detail
                      ? ui("arguments contain “{text}”", { text: summary.detail })
                      : null,
                    weight !== null ? ui("weight {weight}", { weight }) : null,
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                </p>
              </div>
            );
          })
        )}
      </CardContent>
    </Card>
  );
}

function FixturesCard({ draft, onEdit }: { draft: CaseDraft; onEdit: () => void }) {
  const ui = useUiTranslations();
  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between pb-2">
        <CardTitle className="text-sm">{ui("Tool fixtures")}</CardTitle>
        <Button variant="link" size="sm" className="h-auto p-0" onClick={onEdit}>
          {ui("Edit")}
        </Button>
      </CardHeader>
      <CardContent className="divide-y p-0">
        {draft.fixtures.length === 0 ? (
          <p className="px-6 py-4 text-sm text-muted-foreground">
            {ui("None. The agent uses its own tools.")}
          </p>
        ) : (
          draft.fixtures.map((fixture, index) => (
            <div key={index} className="px-6 py-3">
              <p className="font-mono text-sm">{fixture.name || "—"}</p>
              {fixture.description && (
                <p className="text-xs text-muted-foreground">{fixture.description}</p>
              )}
            </div>
          ))
        )}
      </CardContent>
    </Card>
  );
}

function CaseRuns({ evalId, caseId }: { evalId: string; caseId: string }) {
  const ui = useUiTranslations();
  const { data: runs, isLoading: runsLoading } = useEvalRuns(evalId);
  const recentIds = useMemo(() => (runs ?? []).slice(0, RECENT_RUNS).map((run) => run.id), [runs]);
  const { runs: details, isLoading } = useEvalRunComparison(evalId, recentIds);
  const rows = useMemo(
    () =>
      details.flatMap((run) =>
        run.results
          .filter((result) => result.eval_case_id === caseId)
          .map((result) => ({ run, result })),
      ),
    [details, caseId],
  );
  // Keep polling while a run of this case is still going.
  const [, tick] = useState(0);
  useEffect(() => {
    if (!rows.some(({ result }) => result.status === "running" || result.status === "pending"))
      return;
    const timer = setInterval(() => tick((n) => n + 1), 5000);
    return () => clearInterval(timer);
  }, [rows]);

  if (runsLoading || isLoading) return <Skeleton className="h-32 w-full" />;
  if (rows.length === 0) {
    return (
      <p className="py-8 text-center text-sm text-muted-foreground">
        {ui("This case has not run in the latest runs. Try it once to see a result.")}
      </p>
    );
  }
  return (
    <Card>
      <CardContent className="divide-y p-0">
        {rows.map(({ run, result }) => (
          <CaseResultRow key={result.id} evalId={evalId} runId={run.id} result={result} />
        ))}
      </CardContent>
    </Card>
  );
}

function CaseResultRow({
  evalId,
  runId,
  result,
}: {
  evalId: string;
  runId: string;
  result: EvalCaseResult;
}) {
  const ui = useUiTranslations();
  const simulation = simulationOutcome(result);
  return (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-1 px-6 py-3 text-sm">
      <Badge variant={result.status === "passed" ? "default" : "outline"}>{result.status}</Badge>
      <Link href={`/evals/${evalId}/runs/${runId}`} className="font-medium hover:underline">
        {ui("Run {id}", { id: runId.slice(-6) })}
      </Link>
      {result.turns !== undefined && (
        <span className="text-muted-foreground">
          {ui("{count} turns", { count: result.turns })}
        </span>
      )}
      {simulation && (
        <span className="text-muted-foreground">
          {simulation.ended === "done"
            ? ui("the simulated user finished")
            : ui("stopped at the turn limit")}
        </span>
      )}
      {result.error_message && <span className="text-destructive">{result.error_message}</span>}
      {result.session_id && (
        <Link
          href={`/sessions/${result.session_id}`}
          className="ml-auto text-muted-foreground hover:underline"
        >
          {ui("Open session")}
        </Link>
      )}
    </div>
  );
}
