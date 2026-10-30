"use client";
import dynamic from "next/dynamic";
import { useUiTranslations } from "@/providers/locale-provider";
import { useContext, useId } from "react";
import { useParams } from "next/navigation";
import { SessionContext } from "@/app/(main)/sessions/[sessionId]/session-context";
import { StreamdownMessage } from "./streamdown-message";
import "@everruns/generative-ui/styles.css";

const Widgets = dynamic(
  () => import("@everruns/generative-ui").then((module) => module.MessageWidgets),
  { ssr: false },
);

export function WidgetContent({
  text,
  isStreaming = false,
  onMessage,
}: {
  text: string;
  isStreaming?: boolean;
  onMessage?: (text: string) => void | Promise<unknown>;
}) {
  const messageId = useId();
  const ui = useUiTranslations();
  const context = useContext(SessionContext);
  const params = useParams<{ sessionId?: string }>();
  const sessionId = context?.sessionId || params?.sessionId;
  return (
    <div className="console-widget">
      <Widgets
        text={text}
        fallbackLabel={ui("This content could not be displayed.")}
        sendErrorLabel={ui("The message could not be sent.")}
        messageId={messageId}
        streaming={isStreaming || context?.isActive || context?.sendMessage.isPending}
        onMessage={
          onMessage ??
          (sessionId && context && !context.session?.archived_at
            ? (content) => context.sendMessage.mutateAsync({ sessionId, content })
            : undefined)
        }
        renderMarkdown={(content, key) => (
          <StreamdownMessage
            key={key}
            variant="inline"
            className="text-foreground"
            isAnimating={isStreaming}
          >
            {content}
          </StreamdownMessage>
        )}
      />
    </div>
  );
}
