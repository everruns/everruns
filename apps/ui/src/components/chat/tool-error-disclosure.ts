import { createContext, useContext } from "react";

// Who reads a failed tool step decides how much of it to show. Chat (the folded
// work log) is for people using an agent: the row is marked as failed and the raw
// tool error waits behind Details. Session transcripts and the playground, where
// people build and debug agents, print it inline. Public chat never sees tool
// rows at all.
const ToolErrorDisclosureContext = createContext<"inline" | "details">("inline");

export const ToolErrorDisclosureProvider = ToolErrorDisclosureContext.Provider;

export function useToolErrorsInline(): boolean {
  return useContext(ToolErrorDisclosureContext) === "inline";
}
