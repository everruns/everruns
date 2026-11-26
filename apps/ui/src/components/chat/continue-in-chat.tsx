"use client";

import { useRef, useState } from "react";
import { useRouter } from "next/navigation";
import { useQueryClient } from "@tanstack/react-query";
import { ChatIcon } from "@/components/icons/facet-icons";
import { Button } from "@/components/ui/button";
import { useChatMode } from "@/components/chat-mode/chat-mode-context";
import { useAuth } from "@/providers/auth-provider";
import { useOrg } from "@/providers/org-provider";
import { useUiTranslations } from "@/providers/locale-provider";
import { api } from "@/lib/api/client";
import type { Session } from "@/lib/api/types";
import { chatRecoveryEpoch, chatRecoveryKey } from "@/lib/chat-recovery";
import { queueChatHandoff } from "@/lib/chat-handoff";

/** Shared by organization findings and installation diagnostics. */
export interface ChatIssueContext {
  title: string;
  summary: string;
  href: string;
  id: string;
  code?: string;
  status?: string;
  agent?: string | null;
  agent_id?: string | null;
}

export function ContinueInChat({ issue }: { issue: ChatIssueContext }) {
  const ui = useUiTranslations();
  const { user } = useAuth();
  const { currentOrg } = useOrg();
  const mode = useChatMode();
  const router = useRouter();
  const queries = useQueryClient();
  const [opening, setOpening] = useState(false);
  const [error, setError] = useState(false);
  const busy = useRef(false);
  const scope = JSON.stringify([user?.id, currentOrg?.public_id]);
  const current = useRef(scope);
  current.current = scope;
  async function open() {
    if (busy.current || !user?.id || !currentOrg?.public_id) return;
    busy.current = true;
    setOpening(true);
    setError(false);
    const epoch = chatRecoveryEpoch();
    try {
      const { data: chat } = await api.post<Session>("/v1/platform/sessions/platform-chat", {});
      if (current.current !== scope || !chat.id) throw new Error("Chat scope changed");
      const summary = issue.summary
        .slice(0, 6_000)
        .split("\n")
        .map((line) => `> ${line}`)
        .join("\n");
      const text = [
        ui(
          "Help me investigate this issue. Check its current state, explain the cause and suggest what to do next.",
        ),
        issue.title.slice(0, 500),
        summary,
        issue.agent || issue.agent_id
          ? `${ui("Agent")}: ${issue.agent ?? issue.agent_id}`
          : undefined,
        issue.href,
      ]
        .filter(Boolean)
        .join("\n\n");
      queueChatHandoff(chatRecoveryKey(user.id, currentOrg.public_id, chat.id), text, epoch);
      queries.setQueryData(["sessions", "platform-chat", currentOrg.public_id], chat);
      if (mode?.openBeside) mode.openBeside(chat.id);
      else router.push("/chats");
    } catch {
      setError(true);
    } finally {
      busy.current = false;
      setOpening(false);
    }
  }
  return (
    <div className="space-y-1">
      <Button
        type="button"
        size="sm"
        variant="outline"
        disabled={opening || !user?.id || !currentOrg}
        onClick={() => void open()}
      >
        <ChatIcon className="mr-2 size-4" />
        {opening ? ui("Opening chat…") : ui("Continue in chat")}
      </Button>
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {ui("Could not open Platform Chat. Try again.")}
        </p>
      )}
    </div>
  );
}
