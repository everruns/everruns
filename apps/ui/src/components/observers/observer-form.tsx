"use client";

// Shared create/edit form for observers.
//
// Validation: requires a name, at least one scorer, and a non-empty rubric for
// every llm_judge scorer and a non-empty key for every scorer. Server-side
// errors (e.g. a judge model not available to the org) surface in an inline alert.

import { useMemo, useState } from "react";
import { useRouter } from "next/navigation";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Checkbox } from "@/components/ui/checkbox";
import { Notice, NoticeDescription, NoticeTitle } from "@/components/ui/notice";
import { useAgents, useHarnesses } from "@/hooks";
import { useCreateObserver, useUpdateObserver } from "@/hooks";
import { getDisplayName } from "@/lib/entity-lifecycle";
import { parseTagList } from "@/lib/form-validation";
import { ScorerEditor } from "./scorer-editor";
import type {
  Observer,
  ObserverScorerConfig,
  CreateObserverRequest,
  UpdateObserverRequest,
} from "@/lib/api/types";

export interface ObserverFormInitial {
  name?: string;
  description?: string;
  sampling_rate?: number;
  agent_ids?: string[];
  harness_ids?: string[];
  session_tags?: string[];
  scorers?: ObserverScorerConfig[];
}

interface ObserverFormProps {
  /** When set, the form edits an existing observer; otherwise it creates one. */
  observer?: Observer;
  /** Prefill values for the create form (e.g. from "Observe this agent"). */
  initial?: ObserverFormInitial;
}

const DEFAULT_SAMPLING_RATE = 0.1;

function scorerErrors(scorers: ObserverScorerConfig[]): Record<number, string> {
  const errors: Record<number, string> = {};
  scorers.forEach((scorer, index) => {
    if (!scorer.key.trim()) {
      errors[index] = "Key is required";
      return;
    }
    if (scorer.method === "llm_judge" && !scorer.rubric.trim()) {
      errors[index] = "Rubric is required for LLM judge scorers";
    }
  });
  return errors;
}

export function ObserverForm({ observer, initial }: ObserverFormProps) {
  const router = useRouter();
  const createObserver = useCreateObserver();
  const updateObserver = useUpdateObserver();

  const { data: agents = [] } = useAgents();
  const { data: harnesses = [] } = useHarnesses();

  const [name, setName] = useState(observer?.name ?? initial?.name ?? "");
  const [description, setDescription] = useState(
    observer?.description ?? initial?.description ?? "",
  );
  const [samplingRate, setSamplingRate] = useState<number>(
    observer?.sampling_rate ?? initial?.sampling_rate ?? DEFAULT_SAMPLING_RATE,
  );
  const [agentIds, setAgentIds] = useState<string[]>(
    observer?.match.agent_ids ?? initial?.agent_ids ?? [],
  );
  const [harnessIds, setHarnessIds] = useState<string[]>(
    observer?.match.harness_ids ?? initial?.harness_ids ?? [],
  );
  const [sessionTags, setSessionTags] = useState<string>(
    (observer?.match.session_tags ?? initial?.session_tags ?? []).join(", "),
  );
  const [scorers, setScorers] = useState<ObserverScorerConfig[]>(
    observer?.scorers ?? initial?.scorers ?? [],
  );
  const [submitted, setSubmitted] = useState(false);

  const errors = useMemo(() => scorerErrors(scorers), [scorers]);
  const hasScorerErrors = Object.keys(errors).length > 0;
  const nameMissing = !name.trim();
  const noScorers = scorers.length === 0;

  const mutationError = observer ? updateObserver.error : createObserver.error;
  const isPending = observer ? updateObserver.isPending : createObserver.isPending;

  const toggleAgent = (id: string) => {
    setAgentIds((prev) => (prev.includes(id) ? prev.filter((a) => a !== id) : [...prev, id]));
  };

  const toggleHarness = (id: string) => {
    setHarnessIds((prev) => (prev.includes(id) ? prev.filter((h) => h !== id) : [...prev, id]));
  };

  const buildMatch = () => {
    const tags = parseTagList(sessionTags);
    return {
      agent_ids: agentIds.length > 0 ? agentIds : undefined,
      harness_ids: harnessIds.length > 0 ? harnessIds : undefined,
      session_tags: tags.length > 0 ? tags : undefined,
    };
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setSubmitted(true);

    if (nameMissing || noScorers || hasScorerErrors) {
      return;
    }

    const match = buildMatch();

    try {
      if (observer) {
        const request: UpdateObserverRequest = {
          name: name.trim(),
          description: description.trim() || undefined,
          match,
          sampling_rate: samplingRate,
          scorers,
        };
        await updateObserver.mutateAsync({ observerId: observer.id, request });
        router.push(`/observers/${observer.id}`);
      } else {
        const request: CreateObserverRequest = {
          name: name.trim(),
          description: description.trim() || undefined,
          match,
          sampling_rate: samplingRate,
          scorers,
        };
        const created = await createObserver.mutateAsync(request);
        router.push(`/observers/${created.id}`);
      }
    } catch (error) {
      // Surfaced inline via mutationError below.
      console.error("Failed to save observer:", error);
    }
  };

  return (
    <form onSubmit={handleSubmit} className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle>Details</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="observer-name">Name</Label>
            <Input
              id="observer-name"
              placeholder="Production quality monitor"
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
            {submitted && nameMissing && (
              <p className="text-sm text-destructive">Name is required</p>
            )}
          </div>

          <div className="space-y-2">
            <Label htmlFor="observer-description">Description</Label>
            <Textarea
              id="observer-description"
              placeholder="What this observer watches for..."
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              rows={2}
            />
          </div>

          <div className="space-y-2">
            <Label htmlFor="observer-sampling">
              Sampling rate ({Math.round(samplingRate * 100)}% of turns)
            </Label>
            <Input
              id="observer-sampling"
              type="range"
              min={0}
              max={1}
              step={0.05}
              value={samplingRate}
              onChange={(e) => setSamplingRate(Number(e.target.value))}
            />
            <p className="text-xs text-muted-foreground">
              Fraction of matching production turns this observer scores.
            </p>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Match rules</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <p className="text-sm text-muted-foreground">
            Optional filters. When left empty an observer considers all production sessions in the
            org.
          </p>

          <div className="space-y-2">
            <Label>Agents</Label>
            {agents.length === 0 ? (
              <p className="text-sm text-muted-foreground">No agents available.</p>
            ) : (
              <div className="flex max-h-48 flex-col gap-2 overflow-y-auto border p-3">
                {agents.map((agent) => (
                  <label key={agent.id} className="flex cursor-pointer items-center gap-2 text-sm">
                    <Checkbox
                      checked={agentIds.includes(agent.id)}
                      onCheckedChange={() => toggleAgent(agent.id)}
                    />
                    {getDisplayName(agent)}
                  </label>
                ))}
              </div>
            )}
          </div>

          <div className="space-y-2">
            <Label>Harnesses</Label>
            {harnesses.length === 0 ? (
              <p className="text-sm text-muted-foreground">No harnesses available.</p>
            ) : (
              <div className="flex max-h-48 flex-col gap-2 overflow-y-auto border p-3">
                {harnesses.map((harness) => (
                  <label
                    key={harness.id}
                    className="flex cursor-pointer items-center gap-2 text-sm"
                  >
                    <Checkbox
                      checked={harnessIds.includes(harness.id)}
                      onCheckedChange={() => toggleHarness(harness.id)}
                    />
                    {getDisplayName(harness)}
                  </label>
                ))}
              </div>
            )}
          </div>

          <div className="space-y-2">
            <Label htmlFor="observer-tags">Session tags</Label>
            <Input
              id="observer-tags"
              placeholder="comma, separated, tags"
              value={sessionTags}
              onChange={(e) => setSessionTags(e.target.value)}
            />
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Scorers</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <ScorerEditor
            scorers={scorers}
            onChange={setScorers}
            errors={submitted ? errors : undefined}
          />
          {submitted && noScorers && (
            <p className="text-sm text-destructive">Add at least one scorer.</p>
          )}
        </CardContent>
      </Card>

      {mutationError && (
        <Notice variant="destructive">
          <NoticeTitle>Failed to save observer</NoticeTitle>
          <NoticeDescription>{mutationError.message}</NoticeDescription>
        </Notice>
      )}

      <div className="flex gap-4">
        <Button type="submit" disabled={isPending}>
          {isPending ? "Saving..." : observer ? "Save changes" : "Create observer"}
        </Button>
        <Button type="button" variant="outline" onClick={() => router.back()}>
          Cancel
        </Button>
      </div>
    </form>
  );
}
