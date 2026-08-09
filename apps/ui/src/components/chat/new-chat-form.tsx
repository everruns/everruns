/**
 * New chat: pick who you are talking to, then talk. The counterpart is chosen up
 * front because a thread is bound to it for its lifetime — afterwards the
 * binding is shown as a fact, not an editable control
 * (knowledge/ui/information-architecture.md).
 *
 * A thread is an ordinary session: this posts `POST /v1/sessions` and lets the
 * harness be derived from the agent.
 *
 * Platform Chat stays available as a counterpart even though it is a harness
 * rather than an agent. It is the built-in operator chat, and it is how an org
 * with no agents yet creates its first one — dropping it would make the landing
 * surface a dead end on a fresh org.
 */
"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { Loader2, MessageCircle } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ChatErrorAlert } from "@/components/chat/chat-error-alert";
import { useAgents, useHarnesses } from "@/hooks";
import { useCreateSession } from "@/hooks/use-sessions";
import { CHAT_THREAD_TAG, PLATFORM_CHAT_HARNESS_NAME } from "@/lib/chat-threads";
import { getDisplayName } from "@/lib/entity-lifecycle";
import type { CreateSessionRequest } from "@/lib/api/types";

/** Sentinel select value for the harness-bound Platform Chat thread. */
const PLATFORM_CHAT_VALUE = "__platform_chat__";

export function NewChatForm({
  onStartingChange,
}: {
  /**
   * Fired when a thread starts being created, and again with `false` if it
   * fails. Creating a thread invalidates the session list, and a host that swaps
   * this form out on the first result would unmount it before the navigation
   * below runs — so the host holds the form in place until this says otherwise.
   */
  onStartingChange?: (starting: boolean) => void;
} = {}) {
  const router = useRouter();
  const { data: agents = [], isLoading: agentsLoading } = useAgents();
  const { data: harnesses = [] } = useHarnesses();
  const createSession = useCreateSession();
  const [selection, setSelection] = useState("");
  const [error, setError] = useState<string | null>(null);

  const platformChatAvailable = harnesses.some(
    (harness) => harness.name === PLATFORM_CHAT_HARNESS_NAME,
  );

  const start = async () => {
    if (!selection) return;
    setError(null);
    onStartingChange?.(true);
    const binding: Partial<CreateSessionRequest> =
      selection === PLATFORM_CHAT_VALUE
        ? { harness_name: PLATFORM_CHAT_HARNESS_NAME }
        : { agent_id: selection };

    try {
      const session = await createSession.mutateAsync({
        request: { ...binding, tags: [CHAT_THREAD_TAG] } as CreateSessionRequest,
      });
      router.push(`/chats/${session.id}`);
    } catch (e) {
      onStartingChange?.(false);
      setError(e instanceof Error ? e.message : "Could not start the chat.");
    }
  };

  if (!agentsLoading && agents.length === 0 && !platformChatAvailable) {
    return (
      <div className="space-y-3">
        <p className="text-sm text-muted-foreground">
          A chat is a conversation with an agent, so there is nothing to talk to yet.
        </p>
        <Button onClick={() => router.push("/agents/new")}>Create an agent</Button>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center justify-center gap-2">
        <Select value={selection} onValueChange={setSelection} disabled={agentsLoading}>
          <SelectTrigger className="w-64" aria-label="Agent for this chat">
            <SelectValue placeholder={agentsLoading ? "Loading agents..." : "Pick an agent"} />
          </SelectTrigger>
          <SelectContent>
            {platformChatAvailable && (
              <SelectItem value={PLATFORM_CHAT_VALUE}>Platform Chat</SelectItem>
            )}
            {agents.map((agent) => (
              <SelectItem key={agent.id} value={agent.id}>
                {getDisplayName(agent)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Button onClick={start} disabled={!selection || createSession.isPending}>
          {createSession.isPending ? (
            <Loader2 className="size-4 animate-spin" />
          ) : (
            <MessageCircle className="size-4" />
          )}
          Start chat
        </Button>
      </div>
      {error && <ChatErrorAlert message={error} />}
    </div>
  );
}
