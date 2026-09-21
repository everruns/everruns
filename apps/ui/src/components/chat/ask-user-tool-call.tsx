"use client";

import { useEffect, useId, useMemo, useState } from "react";
import { Check, CircleQuestionMark, Clock3, X } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { submitQuestionAnswers, type SubmittedQuestionAnswer } from "@/lib/api/sessions";
import type { ToolCompletedData } from "@/lib/api/types";
import { getFullText } from "@/components/chat/tool-call-utils";

export interface AskUserOption {
  label: string;
  description: string;
  default?: boolean;
}

export interface AskUserQuestion {
  id: string;
  header: string;
  question: string;
  multi_select?: boolean;
  allow_other?: boolean;
  options: AskUserOption[];
}

export interface AskUserArguments {
  questions: AskUserQuestion[];
  timeout_seconds?: number;
  asked_at?: string;
  nudge_at?: string;
  expires_at?: string;
}
export function isAskUserArguments(value: unknown): value is AskUserArguments {
  return (
    typeof value === "object" &&
    value !== null &&
    Array.isArray((value as { questions?: unknown }).questions)
  );
}

interface AskUserResult {
  status: "answered" | "declined" | "cancelled" | "timed_out";
  answered_by: "user" | "timeout" | "unattended";
  answers: SubmittedQuestionAnswer[];
}

interface QuestionSelection {
  selected: string[];
  otherSelected: boolean;
  otherText: string;
}

interface AskUserToolCallProps {
  sessionId: string;
  toolCallId: string;
  request: AskUserArguments;
  requestedAt: string;
  toolResultsMap: Map<string, ToolCompletedData>;
}

function timestamp(value: string | undefined): number | null {
  if (!value) return null;
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function deadlines(request: AskUserArguments, requestedAt: string) {
  const askedAt = timestamp(request.asked_at) ?? timestamp(requestedAt) ?? Date.now();
  const expiresAt =
    timestamp(request.expires_at) ?? askedAt + (request.timeout_seconds ?? 300) * 1000;
  const nudgeAt = timestamp(request.nudge_at) ?? Math.max(askedAt, expiresAt - 60_000);
  return { nudgeAt, expiresAt };
}

function initialSelections(questions: AskUserQuestion[]): Record<string, QuestionSelection> {
  return Object.fromEntries(
    questions.map((question) => [
      question.id,
      {
        selected: question.options.filter((option) => option.default).map((option) => option.label),
        otherSelected: false,
        otherText: "",
      },
    ]),
  );
}

function parseResult(result: ToolCompletedData | undefined): AskUserResult | null {
  const text = getFullText(result?.result);
  if (!text) return null;
  try {
    const parsed = JSON.parse(text) as Partial<AskUserResult>;
    if (
      parsed.status === "answered" ||
      parsed.status === "declined" ||
      parsed.status === "cancelled" ||
      parsed.status === "timed_out"
    ) {
      return {
        status: parsed.status,
        answered_by:
          parsed.answered_by === "timeout" || parsed.answered_by === "unattended"
            ? parsed.answered_by
            : "user",
        answers: Array.isArray(parsed.answers) ? parsed.answers : [],
      };
    }
  } catch {
    return null;
  }
  return null;
}

function answerLabels(result: AskUserResult): string {
  return result.answers
    .flatMap((answer) => [...answer.selected, ...(answer.other_text ? [answer.other_text] : [])])
    .join(", ");
}

function defaultLabels(questions: AskUserQuestion[]): string {
  return questions
    .flatMap((question) => {
      const marked = question.options.filter((option) => option.default);
      const options = marked.length > 0 ? marked : question.options.slice(0, 1);
      return options.map((option) => option.label);
    })
    .join(", ");
}

function formatCountdown(milliseconds: number): string {
  const seconds = Math.max(0, Math.ceil(milliseconds / 1000));
  const minutes = Math.floor(seconds / 60);
  return `${minutes}:${String(seconds % 60).padStart(2, "0")}`;
}

function CompletedAnswer({ result }: { result: AskUserResult }) {
  const labels = answerLabels(result);
  const icon =
    result.status === "answered" || result.status === "timed_out" ? (
      <Check className="h-4 w-4" />
    ) : (
      <X className="h-4 w-4" />
    );
  const text =
    result.status === "answered"
      ? `Answered${labels ? `: ${labels}` : ""}`
      : result.status === "timed_out"
        ? `Auto-selected${labels ? `: ${labels}` : ""}`
        : result.status === "declined"
          ? "Questions declined"
          : "Questions cancelled";

  return (
    <div className="flex items-center gap-2 px-3 py-1.5 text-sm text-muted-foreground">
      {icon}
      <span>{text}</span>
    </div>
  );
}

export function AskUserToolCall({
  sessionId,
  toolCallId,
  request,
  requestedAt,
  toolResultsMap,
}: AskUserToolCallProps) {
  const [selections, setSelections] = useState(() => initialSelections(request.questions));
  const [status, setStatus] = useState<"idle" | "submitting">("idle");
  const [submittedResult, setSubmittedResult] = useState<AskUserResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const labelPrefix = useId();
  const { nudgeAt, expiresAt } = useMemo(
    () => deadlines(request, requestedAt),
    [request, requestedAt],
  );

  const existingToolResult = toolResultsMap.get(toolCallId);
  const existingResult = parseResult(existingToolResult);
  const completedResult = submittedResult ?? existingResult;
  const isClosed = existingToolResult != null || completedResult != null;

  useEffect(() => {
    if (isClosed) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [isClosed]);

  if (completedResult) {
    return <CompletedAnswer result={completedResult} />;
  }
  if (existingToolResult) {
    return (
      <div className="flex items-center gap-2 px-3 py-1.5 text-sm text-muted-foreground">
        <X className="h-4 w-4" />
        Questions closed
      </div>
    );
  }

  const answerFor = (question: AskUserQuestion): SubmittedQuestionAnswer => {
    const selection = selections[question.id];
    return {
      id: question.id,
      selected: selection?.selected ?? [],
      other_text: selection?.otherSelected ? selection.otherText.trim() || null : null,
    };
  };
  const isComplete = request.questions.every((question) => {
    const answer = answerFor(question);
    return answer.selected.length > 0 || answer.other_text != null;
  });

  const selectOption = (question: AskUserQuestion, label: string) => {
    setSelections((current) => {
      const selection = current[question.id];
      const selected = question.multi_select
        ? selection.selected.includes(label)
          ? selection.selected.filter((value) => value !== label)
          : [...selection.selected, label]
        : [label];
      return {
        ...current,
        [question.id]: {
          ...selection,
          selected,
          otherSelected: question.multi_select ? selection.otherSelected : false,
        },
      };
    });
  };

  const selectOther = (question: AskUserQuestion) => {
    setSelections((current) => {
      const selection = current[question.id];
      const otherSelected = !selection.otherSelected;
      return {
        ...current,
        [question.id]: {
          ...selection,
          selected: question.multi_select ? selection.selected : [],
          otherSelected,
        },
      };
    });
  };

  const submit = async (outcome: "answered" | "declined") => {
    setStatus("submitting");
    setError(null);
    const answers = outcome === "answered" ? request.questions.map(answerFor) : [];
    try {
      await submitQuestionAnswers(sessionId, {
        tool_call_id: toolCallId,
        status: outcome,
        answers,
      });
      setSubmittedResult({
        status: outcome,
        answered_by: "user",
        answers,
      });
    } catch {
      setStatus("idle");
      setError("Could not record your answer. Try again.");
    }
  };

  const showCountdown = now >= nudgeAt;
  const defaults = defaultLabels(request.questions);

  return (
    <form
      className="border border-border bg-muted/40"
      onSubmit={(event) => {
        event.preventDefault();
        void submit("answered");
      }}
    >
      <div className="flex items-start gap-3 border-b border-border px-4 py-3">
        <CircleQuestionMark className="mt-0.5 h-5 w-5 shrink-0 text-primary" />
        <div>
          <p className="text-sm font-medium text-foreground">The agent needs your input</p>
          <p className="mt-0.5 text-xs text-muted-foreground">
            Choose an answer to continue the conversation.
          </p>
        </div>
      </div>

      <div className="divide-y divide-border">
        {request.questions.map((question, questionIndex) => {
          const selection = selections[question.id];
          const inputType = question.multi_select ? "checkbox" : "radio";
          const inputName = `ask-user-${toolCallId}-${question.id}`;
          const labelId = `${labelPrefix}-question-${questionIndex}`;
          return (
            <fieldset key={question.id} aria-labelledby={labelId} className="space-y-3 px-4 py-4">
              <div className="flex w-full items-center gap-2">
                <Badge variant="outline">{question.header}</Badge>
                {question.multi_select && (
                  <span className="text-[11px] text-muted-foreground">Select all that apply</span>
                )}
              </div>
              <p id={labelId} className="text-sm font-medium text-foreground">
                {question.question}
              </p>
              <div className="space-y-2">
                {question.options.map((option) => {
                  const checked = selection.selected.includes(option.label);
                  return (
                    <label
                      key={option.label}
                      className="flex cursor-pointer items-start gap-3 border border-border bg-background px-3 py-2.5 hover:border-primary/60"
                    >
                      <input
                        type={inputType}
                        name={inputName}
                        value={option.label}
                        checked={checked}
                        disabled={status === "submitting"}
                        onChange={() => selectOption(question, option.label)}
                        className="mt-1 h-4 w-4 accent-primary"
                      />
                      <span className="min-w-0 flex-1">
                        <span className="flex flex-wrap items-center gap-2 text-sm font-medium text-foreground">
                          {option.label}
                          {option.default && (
                            <Badge variant="accent" className="py-0">
                              Recommended
                            </Badge>
                          )}
                        </span>
                        <span className="mt-0.5 block text-xs leading-5 text-muted-foreground">
                          {option.description}
                        </span>
                      </span>
                    </label>
                  );
                })}

                {question.allow_other && (
                  <div className="border border-border bg-background px-3 py-2.5">
                    <label className="flex cursor-pointer items-center gap-3 text-sm font-medium text-foreground">
                      <input
                        type={inputType}
                        name={inputName}
                        value="other"
                        checked={selection.otherSelected}
                        disabled={status === "submitting"}
                        onChange={() => selectOther(question)}
                        className="h-4 w-4 accent-primary"
                      />
                      Other
                    </label>
                    {selection.otherSelected && (
                      <Input
                        aria-label={`${question.header} other answer`}
                        className="mt-2"
                        placeholder="Type another answer"
                        value={selection.otherText}
                        disabled={status === "submitting"}
                        onChange={(event) =>
                          setSelections((current) => ({
                            ...current,
                            [question.id]: {
                              ...current[question.id],
                              otherText: event.target.value,
                            },
                          }))
                        }
                      />
                    )}
                  </div>
                )}
              </div>
            </fieldset>
          );
        })}
      </div>

      <div className="border-t border-border px-4 py-3">
        {showCountdown && (
          <p className="mb-3 flex items-center gap-1.5 text-xs text-warning">
            <Clock3 className="h-3.5 w-3.5" />
            {now >= expiresAt
              ? `Continuing with ${defaults} now`
              : `Continuing with ${defaults} in ${formatCountdown(expiresAt - now)}`}
          </p>
        )}
        {error && <p className="mb-3 text-xs text-destructive">{error}</p>}
        <div className="flex items-center justify-end gap-2">
          <Button
            type="button"
            size="sm"
            variant="ghost"
            className="text-muted-foreground"
            disabled={status === "submitting"}
            onClick={() => void submit("declined")}
          >
            Decline
          </Button>
          <Button type="submit" size="sm" disabled={status === "submitting" || !isComplete}>
            {status === "submitting" ? "Submitting…" : "Continue"}
          </Button>
        </div>
      </div>
    </form>
  );
}
