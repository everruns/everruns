"use client";

import { useEffect, useState } from "react";
import { usePreviewHarness } from "@/hooks/use-harnesses";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import type { AgentCapabilityConfig, AgentPreviewResponse, ToolDefinition } from "@/lib/api/types";
import { Wrench, FileText, AlertCircle } from "lucide-react";

interface HarnessPreviewProps {
  systemPrompt: string;
  capabilities: AgentCapabilityConfig[];
}

export function HarnessPreview({ systemPrompt, capabilities }: HarnessPreviewProps) {
  const previewMutation = usePreviewHarness();
  const [preview, setPreview] = useState<AgentPreviewResponse | null>(null);

  useEffect(() => {
    previewMutation.mutate(
      {
        system_prompt: systemPrompt,
        capabilities,
      },
      {
        onSuccess: (data) => {
          setPreview(data);
        },
      },
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [systemPrompt, JSON.stringify(capabilities)]);

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
          <ScrollArea className="h-[400px] rounded-md border p-4 bg-muted/50">
            <pre className="text-sm whitespace-pre-wrap font-mono">{preview.system_prompt}</pre>
          </ScrollArea>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Wrench className="w-5 h-5" />
            Available Tools
            <Badge variant="secondary" className="ml-2">
              {preview.tools.length}
            </Badge>
          </CardTitle>
          <CardDescription>
            Tools available from the enabled capabilities
          </CardDescription>
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
    </div>
  );
}

function ToolCard({ tool }: { tool: ToolDefinition }) {
  const [isExpanded, setIsExpanded] = useState(false);

  return (
    <div className="border rounded-lg p-4 bg-card">
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
            <pre className="text-xs p-2 bg-muted rounded-md overflow-x-auto font-mono">
              {JSON.stringify(tool.parameters, null, 2)}
            </pre>
          </ScrollArea>
        )}
      </div>
    </div>
  );
}
