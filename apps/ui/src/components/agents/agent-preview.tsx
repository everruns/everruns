"use client";

import { useEffect, useState } from "react";
import { useAnalyzeAgent, usePreviewAgent } from "@/hooks/use-agents";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import { InitialFilesPreview } from "@/components/files/initial-files-preview";
import type {
  AgentCapabilityConfig,
  AgentFinding,
  AgentPreviewResponse,
  FindingSeverity,
  InitialFile,
  PreviewAgentRequest,
  ToolDefinition,
} from "@/lib/api/types";
import { Wrench, FileText, AlertCircle, ShieldCheck, Sparkles } from "lucide-react";

interface AgentPreviewProps {
  systemPrompt: string;
  capabilities: AgentCapabilityConfig[];
  // Accept missing `initial_files` from agents persisted before that field existed.
  initialFiles: InitialFile[] | null | undefined;
  tools?: ToolDefinition[];
  /** When set, findings with a proposed fix show an Apply button that
   * replaces the located span in the authored system prompt. */
  onApplyFix?: (start: number, end: number, replacement: string) => void;
}

export function AgentPreview({
  systemPrompt,
  capabilities,
  initialFiles,
  tools = [],
  onApplyFix,
}: AgentPreviewProps) {
  const previewMutation = usePreviewAgent();
  const [preview, setPreview] = useState<AgentPreviewResponse | null>(null);

  useEffect(() => {
    // Fetch preview when props change
    previewMutation.mutate(
      {
        system_prompt: systemPrompt,
        capabilities,
        tools,
      },
      {
        onSuccess: (data) => {
          setPreview(data);
        },
      },
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [systemPrompt, JSON.stringify(capabilities), JSON.stringify(tools)]);

  if (previewMutation.isPending && !preview) {
    return (
      <div className="space-y-6">
        <Card>
          <CardHeader>
            <Skeleton className="h-6 w-32" />
          </CardHeader>
          <CardContent>
            <Skeleton className="h-40 w-full" />
          </CardContent>
        </Card>
        <Card>
          <CardHeader>
            <Skeleton className="h-6 w-32" />
          </CardHeader>
          <CardContent>
            <Skeleton className="h-24 w-full" />
          </CardContent>
        </Card>
      </div>
    );
  }

  if (previewMutation.error) {
    return (
      <Card className="border-destructive">
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-destructive">
            <AlertCircle className="w-5 h-5" />
            Preview Error
          </CardTitle>
        </CardHeader>
        <CardContent>
          <p className="text-sm text-muted-foreground">
            Failed to generate preview: {previewMutation.error.message}
          </p>
        </CardContent>
      </Card>
    );
  }

  if (!preview) {
    return null;
  }

  return (
    <div className="space-y-6">
      <FindingsCard
        findings={preview.findings ?? []}
        request={{ system_prompt: systemPrompt, capabilities, tools }}
        onApplyFix={onApplyFix}
      />

      {/* System Prompt Preview */}
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <FileText className="w-5 h-5" />
            Full System Prompt
          </CardTitle>
          <CardDescription>
            The complete system prompt including capability additions
          </CardDescription>
        </CardHeader>
        <CardContent>
          <ScrollArea className="h-[400px] border p-4 bg-muted/50">
            <pre className="text-sm whitespace-pre-wrap font-mono">{preview.system_prompt}</pre>
          </ScrollArea>
        </CardContent>
      </Card>

      {/* Tools Preview */}
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Wrench className="w-5 h-5" />
            Available Tools
            <Badge variant="secondary" className="ml-2">
              {preview.tools.length}
            </Badge>
          </CardTitle>
          <CardDescription>Tools the agent can use from the enabled capabilities</CardDescription>
        </CardHeader>
        <CardContent>
          {preview.tools.length === 0 ? (
            <p className="text-sm text-muted-foreground italic">
              No tools available. Add capabilities to enable tools.
            </p>
          ) : (
            <div className="space-y-4">
              {preview.tools.map((tool, index) => (
                <ToolCard key={index} tool={tool} />
              ))}
            </div>
          )}
        </CardContent>
      </Card>

      <InitialFilesPreview files={initialFiles} />
    </div>
  );
}

const SEVERITY_STYLES: Record<FindingSeverity, string> = {
  warning: "border-amber-500/50 text-amber-600 dark:text-amber-400",
  info: "border-sky-500/50 text-sky-600 dark:text-sky-400",
  suggestion: "border-muted-foreground/50 text-muted-foreground",
};

/** Replace a byte-offset span (as reported by the server) in a JS string. */
function applyByteSpanReplacement(
  text: string,
  start: number,
  end: number,
  replacement: string,
): string {
  const bytes = new TextEncoder().encode(text);
  const decoder = new TextDecoder();
  return decoder.decode(bytes.slice(0, start)) + replacement + decoder.decode(bytes.slice(end));
}

interface FindingsCardProps {
  findings: AgentFinding[];
  request: PreviewAgentRequest;
  onApplyFix?: (start: number, end: number, replacement: string) => void;
}

function FindingsCard({ findings, request, onApplyFix }: FindingsCardProps) {
  const analyzeMutation = useAnalyzeAgent();
  const [analysis, setAnalysis] = useState<AgentFinding[] | null>(null);

  // Analysis results are tied to the config they were computed against;
  // invalidate them when the prompt, capabilities, or tools change (tools
  // feed the server-side llm.tool_guidance checker).
  useEffect(() => {
    setAnalysis(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [request.system_prompt, JSON.stringify(request.capabilities), JSON.stringify(request.tools)]);

  const shown = analysis ?? findings;

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <ShieldCheck className="w-5 h-5" />
          Checks
          {shown.length > 0 && (
            <Badge variant="secondary" className="ml-2">
              {shown.length}
            </Badge>
          )}
          <span className="flex-1" />
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={analyzeMutation.isPending}
            onClick={() =>
              analyzeMutation.mutate(request, {
                onSuccess: (data) => setAnalysis(data.findings),
              })
            }
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

export { applyByteSpanReplacement };

function ToolCard({ tool }: { tool: ToolDefinition }) {
  const [isExpanded, setIsExpanded] = useState(false);

  return (
    <div className="border p-4 bg-card">
      <div className="flex items-start justify-between">
        <div className="space-y-1">
          <h4 className="font-semibold text-sm font-mono">{tool.name}</h4>
          <p className="text-sm text-muted-foreground">{tool.description}</p>
        </div>
        {"policy" in tool && tool.policy === "requires_approval" && (
          <Badge variant="outline" className="text-xs">
            Requires approval
          </Badge>
        )}
      </div>

      {/* Parameters schema */}
      <div className="mt-3">
        <button
          type="button"
          className="text-xs text-muted-foreground hover:text-foreground transition-colors"
          onClick={() => setIsExpanded(!isExpanded)}
        >
          {isExpanded ? "Hide parameters" : "Show parameters"}
        </button>
        {isExpanded && (
          <ScrollArea className="mt-2 max-h-[200px]">
            <pre className="text-xs p-2 bg-muted overflow-x-auto font-mono">
              {JSON.stringify(tool.parameters, null, 2)}
            </pre>
          </ScrollArea>
        )}
      </div>
    </div>
  );
}
