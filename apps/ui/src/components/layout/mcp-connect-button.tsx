"use client";

import { useState, type ReactNode } from "react";
import { ArrowRight, Check, Copy, ExternalLink, SquareTerminal } from "lucide-react";
import { Button, LinkButton } from "@/components/ui/button";
import {
  codingAgentSetupPrompt,
  CODING_AGENT_MANUAL_URL,
  CONNECTED_AI_CLIENTS_HREF,
} from "@/lib/coding-agent-setup";
import { capabilityIconMap } from "@/lib/capability-icons";
import { DropdownMenuItem } from "@/components/ui/dropdown-menu";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { useAuth } from "@/providers/auth-provider";

const McpIcon = capabilityIconMap.mcp;

function useMcpConfig() {
  const mcpUrl = typeof window !== "undefined" ? `${window.location.origin}/mcp` : "/mcp";
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

  return { mcpUrl, configSnippet };
}

function CopySnippetButton({ value, compact = false }: { value: string; compact?: boolean }) {
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState(false);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(value);
      setError(false);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      setError(true);
    }
  };

  return (
    <>
      <Button
        type="button"
        onClick={handleCopy}
        variant={compact ? "ghost" : "default"}
        size={compact ? "icon-sm" : "lg"}
        className={compact ? "absolute top-2 right-2" : "w-full"}
        aria-label={compact ? "Copy MCP configuration" : undefined}
      >
        {copied ? <Check /> : <Copy />}
        {!compact && (copied ? "Copied" : "Copy setup prompt")}
      </Button>
      {error && (
        <p role="status" className="mt-2 text-xs text-destructive">
          Could not copy. Select and copy the text above.
        </p>
      )}
    </>
  );
}

function McpConnectDialogContent({
  mcpUrl,
  configSnippet,
  onNavigate,
}: {
  mcpUrl: string;
  configSnippet: string;
  onNavigate: () => void;
}) {
  const { requiresAuth } = useAuth();

  return (
    <DialogContent className="max-h-[90dvh] overflow-y-auto sm:max-w-xl">
      <DialogHeader>
        <DialogTitle className="flex items-center gap-2">
          <SquareTerminal className="h-5 w-5" />
          Connect your coding agent
        </DialogTitle>
        <DialogDescription>
          Use Everruns from Claude Code, Codex, Cursor, or Gemini CLI.
        </DialogDescription>
      </DialogHeader>

      <div className="space-y-5">
        <div>
          <p className="mb-2 text-sm font-medium">Paste this into your coding agent</p>
          <pre className="mb-3 whitespace-pre-wrap break-words border bg-muted p-4 font-mono text-xs leading-relaxed select-text">
            {codingAgentSetupPrompt(mcpUrl, requiresAuth)}
          </pre>
          <CopySnippetButton value={codingAgentSetupPrompt(mcpUrl, requiresAuth)} />
          <p className="mt-2 text-xs text-muted-foreground">
            Your agent will choose the setup steps for your tools.
          </p>
        </div>

        <div className="space-y-3 border-t pt-4">
          <div>
            <p className="text-sm font-medium">
              {requiresAuth ? "Complete sign-in in your agent" : "No sign-in required"}
            </p>
            <p className="mt-1 text-sm text-muted-foreground">
              {requiresAuth
                ? "Follow the browser OAuth sign-in when prompted."
                : "This local Everruns server does not require authentication."}
            </p>
          </div>
          {requiresAuth && (
            <LinkButton variant="outline" href={CONNECTED_AI_CLIENTS_HREF} onClick={onNavigate}>
              Check connections <ArrowRight />
            </LinkButton>
          )}
          <a
            href={CODING_AGENT_MANUAL_URL}
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center gap-1 text-sm text-primary underline underline-offset-4 sm:ml-3"
          >
            Manual setup <ExternalLink className="size-3" />
          </a>
        </div>

        <details className="border-t pt-4">
          <summary className="cursor-pointer text-sm font-medium">MCP configuration</summary>
          <p className="mt-2 break-all font-mono text-xs text-muted-foreground">{mcpUrl}</p>
          <div className="relative mt-2">
            <pre className="overflow-x-auto border bg-muted px-3 py-2 pr-10 font-mono text-xs">
              {configSnippet}
            </pre>
            <CopySnippetButton value={configSnippet} compact />
          </div>
        </details>
        <p className="border-t pt-3 text-xs text-muted-foreground">
          {requiresAuth && "Your coding agent acts as you across your organisations. "}
          A model provider in Everruns is still needed to run agents.
        </p>
      </div>
    </DialogContent>
  );
}

export function McpConnectDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { mcpUrl, configSnippet } = useMcpConfig();

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <McpConnectDialogContent
        mcpUrl={mcpUrl}
        configSnippet={configSnippet}
        onNavigate={() => onOpenChange(false)}
      />
    </Dialog>
  );
}

export function McpConnectButton() {
  const { mcpUrl, configSnippet } = useMcpConfig();
  const [open, setOpen] = useState(false);

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <Tooltip>
        <TooltipTrigger asChild>
          <DialogTrigger
            className="relative inline-flex h-9 w-9 items-center justify-center border border-transparent text-muted-foreground transition-colors hover:border-border hover:bg-background hover:text-foreground"
            aria-label="Connect your coding agent"
          >
            <McpIcon className="h-4 w-4" />
          </DialogTrigger>
        </TooltipTrigger>
        <TooltipContent>Connect your coding agent</TooltipContent>
      </Tooltip>

      <McpConnectDialogContent
        mcpUrl={mcpUrl}
        configSnippet={configSnippet}
        onNavigate={() => setOpen(false)}
      />
    </Dialog>
  );
}

export function McpConnectMenuItem({ onSelect }: { onSelect: () => void }) {
  return (
    <DropdownMenuItem
      onClick={() => {
        // Let the menu close and restore focus before opening the dialog.
        window.setTimeout(onSelect, 0);
      }}
    >
      <McpIcon className="icon-sharp mr-2 h-4 w-4" />
      Connect your coding agent
    </DropdownMenuItem>
  );
}

/** Shared entry point for onboarding, empty states and connection settings. */
export function CodingAgentConnectAction({
  className,
  children,
}: {
  className?: string;
  children?: ReactNode;
}) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        className={
          className ??
          "inline-flex items-center gap-2 border px-3 py-2 text-sm hover:bg-muted focus-visible:outline-ring"
        }
      >
        {children ?? (
          <>
            <SquareTerminal className="size-4" />
            Connect your coding agent
            <ArrowRight className="size-4" />
          </>
        )}
      </button>
      {open && <McpConnectDialog open={open} onOpenChange={setOpen} />}
    </>
  );
}
