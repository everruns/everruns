"use client";

import { useEffect, useState } from "react";
import { usePreviewAgent } from "@/hooks/use-agents";
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
  ToolDefinition,
} from "@/lib/api/types";
import { Wrench, FileText, AlertCircle, ShieldCheck } from "lucide-react";

interface AgentPreviewProps {
  systemPrompt: string;
  capabilities: AgentCapabilityConfig[];
  // Accept missing `initial_files` from agents persisted before that field existed.
  initialFiles: InitialFile[] | null | undefined;
  tools?: ToolDefinition[];
}

export function AgentPreview({
  systemPrompt,
  capabilities,
  initialFiles,
  tools = [],
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
      <FindingsCard findings={preview.findings ?? []} />

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

function FindingsCard({ findings }: { findings: AgentFinding[] }) {
  if (findings.length === 0) {
    return null;
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <ShieldCheck className="w-5 h-5" />
          Checks
          <Badge variant="secondary" className="ml-2">
            {findings.length}
          </Badge>
        </CardTitle>
        <CardDescription>
          Advisory findings about this configuration. They never block saving.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <ul className="space-y-3">
          {findings.map((finding, index) => (
            <li key={`${finding.rule_id}-${index}`} className="flex items-start gap-3">
              <Badge variant="outline" className={`text-xs ${SEVERITY_STYLES[finding.severity]}`}>
                {finding.severity}
              </Badge>
              <div className="space-y-0.5">
                <p className="text-sm">{finding.message}</p>
                <p className="text-xs text-muted-foreground font-mono">{finding.rule_id}</p>
              </div>
            </li>
          ))}
        </ul>
      </CardContent>
    </Card>
  );
}

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
