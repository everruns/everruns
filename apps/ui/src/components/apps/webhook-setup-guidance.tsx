import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/ui/copy-button";
import { CodeBlock } from "@/components/ui/code-block";
import { getInvocationSessionModeDisplayName } from "@/lib/app-channels";
import { codingAgentPrompt, webhookSamples } from "@/lib/integration/snippets";
import type { InvocationSessionMode } from "@/lib/api/types";
import { Globe, KeyRound } from "lucide-react";

interface WebhookSetupGuidanceProps {
  endpointUrl: string;
  sessionMode: InvocationSessionMode;
  message: string;
  tokenConfigured: boolean;
  isPublished: boolean;
  onConfigure?: () => void;
}

export function WebhookSetupGuidance({
  endpointUrl,
  sessionMode,
  message,
  tokenConfigured,
  isPublished,
  onConfigure,
}: WebhookSetupGuidanceProps) {
  const origin = typeof window !== "undefined" ? window.location.origin : "";
  return (
    <div className="space-y-4">
      <div className="flex items-center gap-2">
        <Badge variant={tokenConfigured ? "default" : "secondary"}>
          {tokenConfigured ? "Token Configured" : "Token Missing"}
        </Badge>
        <span className="text-sm text-muted-foreground">
          {isPublished
            ? "Ready to accept authenticated webhook requests."
            : "Publish the app to accept webhook requests."}
        </span>
      </div>

      <div>
        <p className="text-sm font-medium">Endpoint</p>
        <div className="mt-2 flex items-center gap-2 bg-muted p-3">
          <Globe className="h-4 w-4 shrink-0 text-muted-foreground" />
          <code className="flex-1 truncate text-sm">{endpointUrl}</code>
          <CopyButton value={endpointUrl} />
        </div>
      </div>

      <div className="grid gap-4 md:grid-cols-2">
        <div>
          <p className="text-sm font-medium">Authentication</p>
          <div className="mt-2 flex items-center gap-2 text-sm text-muted-foreground">
            <KeyRound className="h-4 w-4 shrink-0" />
            <span>Use `Authorization: Bearer &lt;token&gt;` or `X-Everruns-Webhook-Token`.</span>
          </div>
        </div>
        <div>
          <p className="text-sm font-medium">Session Mode</p>
          <p className="text-sm text-muted-foreground">
            {getInvocationSessionModeDisplayName(sessionMode)}
          </p>
        </div>
      </div>

      <div>
        <p className="text-sm font-medium">Invocation Message</p>
        <div className="mt-2 rounded-md border p-3 text-sm text-muted-foreground">
          <code className="whitespace-pre-wrap break-words">{message}</code>
        </div>
      </div>

      <div className="space-y-1 text-sm text-muted-foreground">
        <p>Templates can reference payload, webhook headers, and app metadata.</p>
        <p>Each request injects a user message into the app-owned session flow.</p>
      </div>

      <div>
        <p className="text-sm font-medium">Call it from your app</p>
        <div className="mt-2">
          <CodeBlock samples={webhookSamples({ origin, endpointUrl })} />
        </div>
      </div>

      <div>
        <p className="text-sm font-medium">Let a coding agent wire it up</p>
        <div className="mt-2">
          <CodeBlock
            samples={[
              {
                label: "Prompt",
                language: "text",
                code: codingAgentPrompt({ origin, kind: "webhook", endpointUrl }),
              },
            ]}
          />
        </div>
      </div>

      {onConfigure && (
        <div className="pt-1">
          <Button size="sm" variant="outline" onClick={onConfigure}>
            Configure
          </Button>
        </div>
      )}
    </div>
  );
}
