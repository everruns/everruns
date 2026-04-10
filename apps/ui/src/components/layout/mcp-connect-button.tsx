"use client";

import { useState } from "react";
import { Check, Copy } from "lucide-react";
import { capabilityIconMap } from "@/lib/capability-icons";
import { useFeatureFlags } from "@/providers/feature-flags-provider";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";

const McpIcon = capabilityIconMap.mcp;

function CopySnippetButton({ value }: { value: string }) {
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard write can fail (permissions, insecure context) — silently ignore
    }
  };

  return (
    <button
      type="button"
      onClick={handleCopy}
      className="absolute top-2 right-2 inline-flex h-7 w-7 items-center justify-center text-muted-foreground transition-colors hover:text-foreground"
      aria-label={copied ? "Copied" : "Copy to clipboard"}
    >
      {copied ? (
        <Check className="h-3.5 w-3.5 text-green-500" />
      ) : (
        <Copy className="h-3.5 w-3.5" />
      )}
    </button>
  );
}

export function McpConnectButton() {
  const featureFlags = useFeatureFlags();

  if (!featureFlags.mcp_endpoint) {
    return null;
  }

  const mcpUrl =
    typeof window !== "undefined" ? `${window.location.origin}/mcp` : "/mcp";

  const configSnippet = JSON.stringify(
    {
      mcpServers: {
        everruns: {
          url: mcpUrl,
        },
      },
    },
    null,
    2,
  );

  return (
    <Dialog>
      <Tooltip>
        <TooltipTrigger asChild>
          <DialogTrigger
            className="relative inline-flex h-9 w-9 items-center justify-center border border-transparent text-muted-foreground transition-colors hover:border-border hover:bg-background hover:text-foreground"
            aria-label="Connect via MCP"
          >
            <McpIcon className="h-4 w-4" />
          </DialogTrigger>
        </TooltipTrigger>
        <TooltipContent>Connect via MCP</TooltipContent>
      </Tooltip>

      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <McpIcon className="h-5 w-5" />
            Connect via MCP
          </DialogTitle>
          <DialogDescription>
            Use Everruns from Claude Desktop, Cursor, VS Code, or any
            MCP-compatible client.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div>
            <p className="mb-1.5 text-sm font-medium">MCP Server URL</p>
            <div className="relative">
              <code className="block overflow-x-auto border bg-muted px-3 py-2 pr-9 text-sm">
                {mcpUrl}
              </code>
              <CopySnippetButton value={mcpUrl} />
            </div>
          </div>

          <div>
            <p className="mb-1.5 text-sm font-medium">Quick Setup</p>
            <p className="mb-2 text-sm text-muted-foreground">
              Add this to your MCP client config:
            </p>
            <div className="relative">
              <pre className="overflow-x-auto border bg-muted px-3 py-2 pr-9 font-mono text-sm">
                {configSnippet}
              </pre>
              <CopySnippetButton value={configSnippet} />
            </div>
          </div>

          <p className="text-xs text-muted-foreground">
            Authentication is handled automatically via OAuth when connecting
            from an MCP client.
          </p>
        </div>
      </DialogContent>
    </Dialog>
  );
}
