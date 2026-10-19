"use client";

// What a capability config editor may need to know about the agent it edits,
// beyond its own config. The agent page provides the agent's default model so
// the Web search editor can say how this agent will search; editors outside an
// agent (harnesses) get nothing and fall back to the organization's default.

import { createContext, useContext } from "react";

export interface CapabilityEditorContextValue {
  /** The agent's default model id; empty means the organization's default model. */
  modelId?: string;
}

export const CapabilityEditorContext = createContext<CapabilityEditorContextValue>({});

export function useCapabilityEditorContext(): CapabilityEditorContextValue {
  return useContext(CapabilityEditorContext);
}
