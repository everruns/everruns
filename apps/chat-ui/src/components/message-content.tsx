"use client";
import dynamic from "next/dynamic";
import { useId } from "react";
import { Streamdown } from "streamdown";
import { artifactPath } from "./artifact-path";
import "@everruns/generative-ui/styles.css";

const Widgets = dynamic(
  () => import("@everruns/generative-ui").then((module) => module.MessageWidgets),
  { ssr: false },
);

export function MessageContent({
  text,
  streaming = false,
  onMessage,
  onFile,
}: {
  text: string;
  streaming?: boolean;
  onMessage?: (text: string) => void | Promise<unknown>;
  onFile?: (path: string) => void;
}) {
  const messageId = useId();
  const markdown = (content: string, key: string) => (
    <Streamdown
      key={key}
      components={{
        a: ({ href, children, ...props }) => {
          const path = onFile ? artifactPath(href) : null;
          return path ? (
            <button className="artifact-link" onClick={() => onFile?.(path)}>
              {children}
            </button>
          ) : (
            <a {...props} href={href}>
              {children}
            </a>
          );
        },
      }}
    >
      {content}
    </Streamdown>
  );
  return /^(?:`{3,}|~{3,})(?:openui|a2ui)(?:\s|$)/m.test(text) ? (
    <Widgets
      text={text}
      messageId={messageId}
      streaming={streaming}
      onMessage={onMessage}
      renderMarkdown={markdown}
    />
  ) : (
    markdown(text, "text")
  );
}
