"use client";

import { useState, type ReactNode } from "react";
import { ArrowRight, Check, Copy, ExternalLink, SquareTerminal } from "lucide-react";
import { Button } from "@/components/ui/button";
import { codingAgentSetupPrompt, CODING_AGENT_MANUAL_URL } from "@/lib/coding-agent-setup";
import { CodingAgentIcons } from "@/components/icons/coding-agent-icons";
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

function CopySnippetButton({ value }: { value: string }) {
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
      <Button type="button" onClick={handleCopy} variant="default" size="lg" className="w-full">
        {copied ? <Check /> : <Copy />}
        {copied ? "Copied" : "Copy setup prompt"}
      </Button>
      {error && (
        <p role="status" className="mt-2 text-xs text-destructive">
          Could not copy. Select and copy the text above.
        </p>
      )}
    </>
  );
}

function McpConnectDialogContent() {
  const { requiresAuth } = useAuth();
  const mcpUrl = typeof window !== "undefined" ? `${window.location.origin}/mcp` : "/mcp";
  const prompt = codingAgentSetupPrompt(mcpUrl, requiresAuth);

  return (
    <DialogContent className="max-h-[90dvh] overflow-y-auto sm:max-w-lg">
      <DialogHeader className="text-left">
        <DialogTitle>Connect your coding agent</DialogTitle>
        <DialogDescription>Copy this prompt and paste it into your coding agent.</DialogDescription>
      </DialogHeader>

      <CodingAgentIcons />

      <div>
        <pre className="mb-3 whitespace-pre-wrap break-words border bg-muted p-4 font-mono text-xs leading-relaxed select-text">
          {prompt}
        </pre>
        <CopySnippetButton value={prompt} />
      </div>

      <a
        href={CODING_AGENT_MANUAL_URL}
        target="_blank"
        rel="noopener noreferrer"
        className="mx-auto inline-flex items-center gap-1 text-xs text-muted-foreground underline underline-offset-4 hover:text-foreground"
      >
        Manual setup <ExternalLink className="size-3" />
      </a>
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
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <McpConnectDialogContent />
    </Dialog>
  );
}

export function McpConnectButton() {
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

      <McpConnectDialogContent />
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
