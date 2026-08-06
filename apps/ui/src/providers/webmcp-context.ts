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

const unavailableContext: WebMcpContextValue = {
  enabled: false,
  bindingToken: "unbound",
  assertBinding: () => {
    throw new DOMException("WebMCP is unavailable outside its provider", "AbortError");
  },
  requestApproval: () =>
    Promise.reject(new DOMException("WebMCP is unavailable outside its provider", "AbortError")),
};

// Keep feature consumers inert when rendered in isolation (for example, component tests).
// The provider replaces this value in the authenticated application layout.
export const WebMcpContext = createContext<WebMcpContextValue>(unavailableContext);

export function useWebMcp() {
  return useContext(WebMcpContext);
}
