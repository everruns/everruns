"use client";

import { useEffect, useRef, useState } from "react";
import { useAnalyzeAgent, usePreviewAgent } from "@/hooks/use-agents";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import type {
  AgentCapabilityConfig,
  AgentFinding,
  FindingSeverity,
  PreviewAgentRequest,
  ToolDefinition,
} from "@/lib/api/types";
import { AlertCircle, ShieldCheck, Sparkles } from "lucide-react";

const CHECK_REFRESH_DELAY_MS = 300;

interface AgentChecksProps {
  systemPrompt: string;
  capabilities: AgentCapabilityConfig[];
  tools?: ToolDefinition[];
  /** Applies a server-proposed byte span replacement to the authored system prompt. */
  onApplyFix?: (start: number, end: number, replacement: string) => void;
}

export function AgentChecks({
  systemPrompt,
  capabilities,
  tools = [],
  onApplyFix,
}: AgentChecksProps) {
  const previewMutation = usePreviewAgent();
  const request: PreviewAgentRequest = {
    system_prompt: systemPrompt,
    capabilities,
    tools,
  };
  const requestKey = JSON.stringify(request);
  const latestRequestKey = useRef(requestKey);
  const hasRequestedChecks = useRef(false);
  latestRequestKey.current = requestKey;
  const [result, setResult] = useState<{ requestKey: string; findings: AgentFinding[] } | null>(
    null,
  );
  const findings = result?.requestKey === requestKey ? result.findings : null;

  useEffect(() => {
    setResult(null);
    previewMutation.reset();
    const timeout = window.setTimeout(
      () => {
        hasRequestedChecks.current = true;
        previewMutation.mutate(request, {
          onSuccess: (data) => {
            if (latestRequestKey.current === requestKey) {
              setResult({ requestKey, findings: data.findings ?? [] });
            }
          },
        });
      },
      hasRequestedChecks.current ? CHECK_REFRESH_DELAY_MS : 0,
    );

    return () => window.clearTimeout(timeout);
    // requestKey captures every field sent to the preview endpoint. Depending on
    // the object itself would refresh on every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [requestKey]);

  if (previewMutation.error) {
    return (
      <Card className="border-destructive">
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-destructive">
            <AlertCircle className="w-5 h-5" />
            Checks Error
          </CardTitle>
        </CardHeader>
        <CardContent>
          <p className="text-sm text-muted-foreground">
            Failed to run checks: {previewMutation.error.message}
          </p>
        </CardContent>
      </Card>
    );
  }

  if (findings === null) {
    return <ChecksSkeleton />;
  }

  return (
    <FindingsCard
      findings={findings}
      request={request}
      requestKey={requestKey}
      onApplyFix={onApplyFix}
    />
  );
}

function ChecksSkeleton() {
  return (
    <Card aria-label="Refreshing checks">
      <CardHeader>
        <Skeleton className="h-6 w-32" />
      </CardHeader>
      <CardContent>
        <Skeleton className="h-24 w-full" />
      </CardContent>
    </Card>
  );
}

const SEVERITY_STYLES: Record<FindingSeverity, string> = {
  warning: "border-amber-500/50 text-amber-600 dark:text-amber-400",
  info: "border-sky-500/50 text-sky-600 dark:text-sky-400",
  suggestion: "border-muted-foreground/50 text-muted-foreground",
};

interface FindingsCardProps {
  findings: AgentFinding[];
  request: PreviewAgentRequest;
  requestKey: string;
  onApplyFix?: (start: number, end: number, replacement: string) => void;
}

function FindingsCard({ findings, request, requestKey, onApplyFix }: FindingsCardProps) {
  const analyzeMutation = useAnalyzeAgent();
  const latestRequestKey = useRef(requestKey);
  latestRequestKey.current = requestKey;
  const [analysis, setAnalysis] = useState<AgentFinding[] | null>(null);

  useEffect(() => {
    setAnalysis(null);
    analyzeMutation.reset();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [requestKey]);

  const shown = analysis ?? findings;

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex flex-wrap items-center gap-2">
          <ShieldCheck className="w-5 h-5" />
          Checks
          {shown.length > 0 && <Badge variant="secondary">{shown.length}</Badge>}
          <span className="flex-1" />
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={analyzeMutation.isPending}
            onClick={() => {
              const analyzedRequestKey = requestKey;
              analyzeMutation.mutate(request, {
                onSuccess: (data) => {
                  if (latestRequestKey.current === analyzedRequestKey) {
                    setAnalysis(data.findings);
                  }
                },
              });
            }}
          >
            <Sparkles className="w-4 h-4 mr-1" />
            {analyzeMutation.isPending ? "Analyzing…" : "Analyze"}
          </Button>
        </CardTitle>
        <CardDescription>
          Advisory findings about this configuration. They never block saving. Analyze runs a deeper
          AI review (takes ~30s).
        </CardDescription>
      </CardHeader>
      <CardContent>
        {analyzeMutation.error && (
          <p className="text-sm text-destructive mb-3">
            Analysis failed: {analyzeMutation.error.message}
          </p>
        )}
        {shown.length === 0 ? (
          <p className="text-sm text-muted-foreground italic">
            {analysis
              ? "No issues found by built-in rules or AI analysis."
              : "No issues found by built-in rules."}
          </p>
        ) : (
          <ul className="space-y-3">
            {shown.map((finding, index) => (
              <FindingRow
                key={`${finding.rule_id}-${index}`}
                finding={finding}
                onApplyFix={onApplyFix}
              />
            ))}
          </ul>
        )}
      </CardContent>
    </Card>
  );
}

function FindingRow({
  finding,
  onApplyFix,
}: {
  finding: AgentFinding;
  onApplyFix?: (start: number, end: number, replacement: string) => void;
}) {
  const fixSpan =
    finding.fix !== undefined &&
    finding.location?.field === "system_prompt" &&
    finding.location.start !== undefined &&
    finding.location.end !== undefined
      ? { start: finding.location.start, end: finding.location.end }
      : null;

  return (
    <li className="flex items-start gap-3">
      <Badge variant="outline" className={`text-xs ${SEVERITY_STYLES[finding.severity]}`}>
        {finding.severity}
      </Badge>
      <div className="space-y-1 min-w-0">
        <p className="text-sm">{finding.message}</p>
        <p className="text-xs text-muted-foreground font-mono">{finding.rule_id}</p>
        {finding.fix !== undefined && (
          <div className="text-xs border p-2 bg-muted/50 space-y-1">
            <p className="text-muted-foreground">Suggested replacement:</p>
            <pre className="whitespace-pre-wrap font-mono">{finding.fix}</pre>
            {fixSpan && onApplyFix && (
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => onApplyFix(fixSpan.start, fixSpan.end, finding.fix ?? "")}
              >
                Apply fix
              </Button>
            )}
          </div>
        )}
      </div>
    </li>
  );
}

/** Replace a byte-offset span (as reported by the server) in a JS string. */
export function applyByteSpanReplacement(
  text: string,
  start: number,
  end: number,
  replacement: string,
): string {
  const bytes = new TextEncoder().encode(text);
  const decoder = new TextDecoder();
  return decoder.decode(bytes.slice(0, start)) + replacement + decoder.decode(bytes.slice(end));
}
