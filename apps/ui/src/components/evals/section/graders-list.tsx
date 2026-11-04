"use client";

// Graders tab: the checks a dataset case can use, grouped by how they decide.
// The list mirrors the server's `Scorer` variants
// (crates/server/src/domains/evals/graders); `graders-list.test.tsx` fails
// when the generated API types gain a variant this list does not describe.

import { useUiTranslations } from "@/providers/locale-provider";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { OpenApiScorer } from "@/lib/api/types";

export type GraderType = OpenApiScorer["type"];

export interface GraderGroup {
  title: string;
  description: string;
  graders: { type: GraderType; label: string; description: string }[];
}

export const GRADER_GROUPS: GraderGroup[] = [
  {
    title: "Answer checks",
    description: "Exact and repeatable. Free to run.",
    graders: [
      {
        type: "contains",
        label: "Contains",
        description: "The final answer contains a phrase.",
      },
      {
        type: "not_contains",
        label: "Does not contain",
        description: "The final answer leaves a phrase out.",
      },
      {
        type: "regex",
        label: "Matches pattern",
        description: "The final answer matches a regular expression.",
      },
      {
        type: "json_schema",
        label: "JSON schema",
        description: "The final answer is JSON that fits a schema.",
      },
      {
        type: "file_contains",
        label: "File contains",
        description: "A file the agent wrote contains a phrase.",
      },
    ],
  },
  {
    title: "Tool use",
    description: "What the agent did, not only what it said.",
    graders: [
      {
        type: "tool_called",
        label: "Tool called",
        description: "The agent called a tool at least so many times.",
      },
      {
        type: "tool_not_called",
        label: "Tool not called",
        description: "The agent never called a tool.",
      },
      {
        type: "tool_call_count",
        label: "Tool call count",
        description: "Total tool calls stayed within a range.",
      },
    ],
  },
  {
    title: "Cost and length",
    description: "Catch answers that got right by spending too much.",
    graders: [
      {
        type: "turns_within",
        label: "Turns within",
        description: "The case finished within so many turns.",
      },
      {
        type: "tokens_within",
        label: "Tokens within",
        description: "The agent spent at most so many tokens.",
      },
    ],
  },
  {
    title: "Model graded",
    description: "For what code cannot check. Grading cost counts toward the run's cap.",
    graders: [
      {
        type: "decision",
        label: "Decision",
        description:
          "A decision model answers one yes or no question about the answer. Cheap and typed.",
      },
      {
        type: "judge",
        label: "Judge",
        description: "The org's chat model grades the answer against a rubric, with its reasoning.",
      },
      {
        type: "citation_faithful",
        label: "Citations verified",
        description: "Cited sources cover the claims and were verified as supporting them.",
      },
      {
        type: "citation_judged",
        label: "Citations judged",
        description: "A model grades each cited claim against its source.",
      },
    ],
  },
];

export function GradersList() {
  const ui = useUiTranslations();
  return (
    <div className="grid gap-4 lg:grid-cols-2">
      {GRADER_GROUPS.map((group) => (
        <Card key={group.title}>
          <CardHeader>
            <CardTitle className="text-sm">{ui(group.title)}</CardTitle>
            <p className="text-xs text-muted-foreground">{ui(group.description)}</p>
          </CardHeader>
          <CardContent className="p-0">
            <ul className="divide-y">
              {group.graders.map((grader) => (
                <li key={grader.type} className="space-y-0.5 px-4 py-2.5">
                  <p className="text-sm font-medium">
                    {ui(grader.label)}
                    <span className="ml-2 font-mono text-[11px] font-normal text-muted-foreground">
                      {grader.type}
                    </span>
                  </p>
                  <p className="text-xs text-muted-foreground">{ui(grader.description)}</p>
                </li>
              ))}
            </ul>
          </CardContent>
        </Card>
      ))}
    </div>
  );
}
