"use client";

import { createContext, useContext } from "react";

export interface WebMcpApprovalRequest {
  title: string;
  description: string;
  confirmLabel: string;
  destructive?: boolean;
}

export interface WebMcpContextValue {
  enabled: boolean;
  bindingToken: string;
  assertBinding: (token: string) => void;
  requestApproval: (request: WebMcpApprovalRequest) => Promise<void>;
}

export const WebMcpContext = createContext<WebMcpContextValue | null>(null);

export function useWebMcp() {
  const context = useContext(WebMcpContext);
  if (!context) {
    throw new Error("useWebMcp must be used within WebMcpProvider");
  }
  return context;
}
