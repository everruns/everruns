"use client";

// Generic, provider-agnostic transcript view. Renders the normalized transcript
// an external eval system (e.g. Mira) attaches to an imported result, plus its
// open-vocab metrics bag. Deliberately source-agnostic: it reads the normalized
// shape, not any one provider's event schema, so the same component can grow to
// render native everruns sessions too.

import { Badge } from "@/components/ui/badge";
import type { EvalTranscript } from "@/lib/api/types";

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="mb-3 last:mb-0">
      <p className="text-xs font-medium text-muted-foreground mb-1">{title}</p>
      {children}
    </div>
  );
}

function formatMetric(value: number): string {
  if (!Number.isFinite(value)) return String(value);
  if (Number.isInteger(value)) return value.toLocaleString();
  // Small fractions (cost, rates) keep precision; larger values round.
  return Math.abs(value) < 1 ? value.toPrecision(3) : value.toLocaleString();
}

export function TranscriptView({
  transcript,
  metrics,
}: {
  transcript?: EvalTranscript;
  metrics?: Record<string, number>;
}) {
  const hasTranscript =
    transcript &&
    ((transcript.input?.length ?? 0) > 0 ||
      !!transcript.final_response ||
      (transcript.tool_calls?.length ?? 0) > 0 ||
      (transcript.output?.length ?? 0) > 0);
  const metricEntries = metrics ? Object.entries(metrics) : [];

  if (!hasTranscript && metricEntries.length === 0) {
    return <p className="text-xs text-muted-foreground">No transcript captured for this result.</p>;
  }

  return (
    <div className="rounded-md border bg-muted/30 p-3 text-sm">
      {transcript?.input && transcript.input.length > 0 && (
        <Section title="Input">
          {transcript.input.map((turn, i) => (
            <div key={i} className="mb-1 whitespace-pre-wrap rounded bg-background p-2 text-sm">
              {turn}
            </div>
          ))}
        </Section>
      )}

      {transcript?.final_response && (
        <Section title="Final response">
          <div className="whitespace-pre-wrap rounded bg-background p-2 text-sm">
            {transcript.final_response}
          </div>
        </Section>
      )}

      {transcript?.tool_calls && transcript.tool_calls.length > 0 && (
        <Section title={`Tool calls (${transcript.tool_calls.length})`}>
          <div className="flex flex-wrap gap-1">
            {transcript.tool_calls.map((tool, i) => (
              <Badge key={`${tool}-${i}`} variant="outline" className="text-xs">
                {tool}
              </Badge>
            ))}
          </div>
        </Section>
      )}

      {transcript?.output && transcript.output.length > 0 && (
        <Section title="Output parts">
          <pre className="overflow-x-auto rounded bg-background p-2 text-xs">
            {JSON.stringify(transcript.output, null, 2)}
          </pre>
        </Section>
      )}

      {metricEntries.length > 0 && (
        <Section title="Metrics">
          <div className="flex flex-wrap gap-1">
            {metricEntries.map(([key, value]) => (
              <Badge key={key} variant="secondary" className="text-xs">
                {key}: {formatMetric(value)}
              </Badge>
            ))}
          </div>
        </Section>
      )}
    </div>
  );
}
