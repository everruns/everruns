"use client";
import { createContext, useContext } from "react";

export const ChatWorkspaceContext = createContext<{
  sessionId: string;
  openTask: (taskId: string) => void;
} | null>(null);
export const useChatWorkspace = () => useContext(ChatWorkspaceContext);
