"use client";

// Chat-only MCP servers: servers a person asked the agent to add "for this chat
// only" (user_mcp add with scope "chat"). They live on the session, not in the
// person's list, so this is the one place to see and remove them outside the
// chat itself. See knowledge/integrations/user-mcp-servers.md (D6).
//
// The header button appears only once the chat has such a server: most chats
// never do, and an empty control would only add noise next to Pin/Archive.

import { useEffect, useState } from "react";
import { Trash2 } from "lucide-react";
import { capabilityIconMap } from "@/lib/capability-icons";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { ChatErrorAlert } from "@/components/chat/chat-error-alert";
import { useChatMcpServers, useRemoveChatMcpServer } from "@/hooks/use-chat-mcp-servers";
import type { ChatMcpServer } from "@/lib/api/types";

const McpIcon = capabilityIconMap.mcp;

function host(url: string) {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

function SignInBadge({ server }: { server: ChatMcpServer }) {
  switch (server.connection) {
    case "connected":
      return <Badge variant="success">Signed in</Badge>;
    case "not_connected":
      return <Badge variant="outline">Needs sign-in</Badge>;
    default:
      return null;
  }
}

/** The chat-only servers of one session, each with a remove action. */
export function ChatMcpServersList({
  sessionId,
  readOnly = false,
}: {
  sessionId: string;
  readOnly?: boolean;
}) {
  const { data: servers = [], isLoading, error } = useChatMcpServers(sessionId);
  const remove = useRemoveChatMcpServer(sessionId);

  if (isLoading) return <p className="text-sm text-muted-foreground">Loading…</p>;
  if (error) return <ChatErrorAlert message={error.message} />;
  if (servers.length === 0) {
    return <p className="text-sm text-muted-foreground">No MCP servers were added to this chat.</p>;
  }
  return (
    <div className="space-y-3">
      <ul className="divide-y divide-border border border-border">
        {servers.map((server) => (
          <li key={server.name} className="flex items-center gap-3 px-3 py-2">
            <div className="min-w-0 flex-1">
              <div className="flex flex-wrap items-center gap-2">
                <span className="truncate font-mono text-sm">{server.name}</span>
                {server.catalog_name && <Badge variant="secondary">Catalog</Badge>}
                <SignInBadge server={server} />
              </div>
              <div className="truncate text-xs text-muted-foreground">{host(server.url)}</div>
            </div>
            {!readOnly && (
              <Button
                variant="ghost"
                size="icon"
                aria-label={`Remove ${server.name}`}
                disabled={remove.isPending}
                onClick={() => remove.mutate(server.name)}
              >
                <Trash2 />
              </Button>
            )}
          </li>
        ))}
      </ul>
      {remove.error && <ChatErrorAlert message={remove.error.message} />}
    </div>
  );
}

/** Header action: shown only when the chat has chat-only MCP servers. */
export function ChatMcpServersButton({
  sessionId,
  status,
  readOnly = false,
}: {
  sessionId: string;
  /** Session status; a finished turn may have added or removed a server. */
  status?: string;
  readOnly?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const { data: servers = [], refetch } = useChatMcpServers(sessionId);

  useEffect(() => {
    if (status === "idle") void refetch();
  }, [status, refetch]);

  if (servers.length === 0) return null;
  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <Button
        variant="outline"
        size="sm"
        aria-label={`MCP servers in this chat (${servers.length})`}
        className="max-sm:w-7 max-sm:px-0"
        onClick={() => setOpen(true)}
      >
        <McpIcon className="size-4" />
        <span className="max-sm:hidden">MCP {servers.length}</span>
      </Button>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>MCP servers in this chat</DialogTitle>
          <DialogDescription>
            Added for this chat only: they join every later turn here and no other chat. Removing
            one drops its tools from the next turn.
          </DialogDescription>
        </DialogHeader>
        <ChatMcpServersList sessionId={sessionId} readOnly={readOnly} />
      </DialogContent>
    </Dialog>
  );
}
