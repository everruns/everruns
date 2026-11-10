"use client";
import { createContext, useContext } from "react";
import type { SessionTask } from "@/lib/api/types";

export const ChatWorkspaceContext = createContext<{
  sessionId: string;
  /** A thread opens in the Threads panel (or the page pane beside a page). */
  openTask: (task: SessionTask) => void;
} | null>(null);
export const useChatWorkspace = () => useContext(ChatWorkspaceContext);
