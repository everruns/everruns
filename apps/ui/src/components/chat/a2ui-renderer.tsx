"use client";
import { WidgetContent } from "./widget-content";

export function A2UIBlock({ code, isStreaming = false }: { code: string; isStreaming?: boolean }) {
  if (!code.trim()) return null;
  return (
    <WidgetContent
      text={"```a2ui\n" + code + (isStreaming ? "" : "\n``` ")}
      isStreaming={isStreaming}
    />
  );
}
