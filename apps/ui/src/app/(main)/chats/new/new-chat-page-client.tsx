"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { useRef } from "react";
import { useRouter } from "next/navigation";
import { useOrganization } from "@/hooks/use-organizations";
import { useAgent } from "@/hooks/use-agents";
import { useModels, useDefaultModel } from "@/hooks/use-providers";
import { useCreateSession } from "@/hooks/use-sessions";
import { useOrg } from "@/providers/org-provider";
import { SessionProvider } from "@/app/(main)/sessions/[sessionId]/session-context";
import { ChatPanel } from "@/components/chat/chat-panel";
import { ChatErrorAlert } from "@/components/chat/chat-error-alert";
import { Skeleton } from "@/components/ui/skeleton";
import { sendUserMessageWithImages } from "@/lib/api/messages";
import { isChatModel } from "@/lib/model-capabilities";
import type { Controls } from "@/lib/api/types";
import { PLATFORM_CHAT_AGENT_NAME, CHAT_THREAD_TAG } from "@/lib/chat-threads";

export function ChatDraft({ threadMode = false }: { threadMode?: boolean }) {
  const ui = useUiTranslations();
  const router = useRouter();
  const { data: agent, error } = useAgent(PLATFORM_CHAT_AGENT_NAME);
  const { data: models = [] } = useModels();
  const { data: defaultModel } = useDefaultModel();
  const {
    data: organization,
    isLoading: organizationLoading,
    error: organizationError,
  } = useOrganization();
  const createSession = useCreateSession();
  // A lost/failed first message must reuse the already-created session on retry.
  const sessionId = useRef<string | null>(null);
  const sending = useRef(false);
  const configuredModel =
    agent?.default_model_id ?? organization?.chat_model_id ?? defaultModel?.id;
  const draftModel = models.find(
    (m) => m.id === configuredModel && m.enabled && m.healthy && isChatModel(m),
  );
  const submit = async (
    text: string,
    images: Array<{ imageId: string; filename?: string }>,
    files: Array<{ fileId: string; filename?: string }>,
    controls?: Controls,
  ) => {
    if (sending.current) return;
    sending.current = true;
    try {
      if (!sessionId.current) {
        const session = await createSession.mutateAsync({
          request: {
            source: "chat",
            agent_name: PLATFORM_CHAT_AGENT_NAME,
            tags: [CHAT_THREAD_TAG],
          },
        });
        sessionId.current = session.id;
      }
      await sendUserMessageWithImages(sessionId.current, text, images, controls, undefined, files);
      router.replace(`/chats/${sessionId.current}`);
    } finally {
      sending.current = false;
    }
  };
  if (error || organizationError)
    return <ChatErrorAlert message={(error || organizationError)!.message} />;
  if (!agent || organizationLoading) return <Skeleton className="m-6 h-64" />;
  return (
    <SessionProvider sessionId="" draftAgent={agent} draftModel={draftModel}>
      <div className="flex h-full flex-col bg-background bg-brand-dots">
        <div className="border-b border-border/70 bg-background/70 px-6 py-3">
          <h1 className="text-base font-semibold">
            {threadMode ? ui("New thread") : ui("New subchat")}
          </h1>
          <p className="text-xs text-muted-foreground">
            {ui("A fresh conversation to manage your Everruns organization.")}
          </p>
        </div>
        <ChatPanel
          showParticipants={false}
          replyToLabel="Platform Chat"
          platformIcon="everruns"
          platformIntro={agent.intro_markdown}
          platformStarters={agent.starters ?? []}
          onDraftSubmit={submit}
        />
      </div>
    </SessionProvider>
  );
}
export default function NewChatPageClient() {
  const { currentOrg } = useOrg();
  return <ChatDraft key={currentOrg?.public_id} />;
}
