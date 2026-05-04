"use client";

import {
  SessionContext,
  type SessionContextValue,
} from "@/app/(main)/sessions/[sessionId]/session-context";
import { SessionLayoutContent } from "@/app/(main)/sessions/[sessionId]/layout";

export function DevSessionRuntimeFrame({
  contextValue,
  children,
}: {
  contextValue: SessionContextValue;
  children: React.ReactNode;
}) {
  return (
    <SessionContext.Provider value={contextValue}>
      <div className="overflow-hidden border border-border/70 bg-background shadow-[0_1px_0_hsl(var(--background)/0.92)]">
        <div className="flex min-h-[760px] flex-col">
          <SessionLayoutContent sessionId={contextValue.sessionId}>{children}</SessionLayoutContent>
        </div>
      </div>
    </SessionContext.Provider>
  );
}
