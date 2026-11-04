"use client";
import { WidgetContent } from "./widget-content";

export function OpenUIBlock({
  code,
  isStreaming = false,
}: {
  code: string;
  isStreaming?: boolean;
}) {
  if (!code.trim()) return null;
  return (
    <WidgetContent
      text={"```openui\n" + code + (isStreaming ? "" : "\n``` ")}
      isStreaming={isStreaming}
    />
  );
}
