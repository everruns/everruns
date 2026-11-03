"use client";

import { use } from "react";
import { SessionProvider } from "./session-context";
import { SessionLayoutContent } from "@/components/session/session-layout-content";

interface SessionLayoutProps {
  children: React.ReactNode;
  params: Promise<{ sessionId: string }>;
}

export default function SessionLayout({ children, params }: SessionLayoutProps) {
  const { sessionId } = use(params);

  return (
    <SessionProvider sessionId={sessionId}>
      <SessionLayoutContent sessionId={sessionId}>{children}</SessionLayoutContent>
    </SessionProvider>
  );
}
