"use client";

import { useState, type Dispatch, type SetStateAction } from "react";
import type { ChannelFormState } from "@/components/agents/channels/channel-form";
import { useTextPageFormDraft } from "./use-text-page-form-draft";

const fieldsByKind = {
  public_chat: ["publicChatDisplayName", "publicChatWelcomeMessage", "publicChatGenericToolText"],
  schedule: ["scheduleCronExpression", "scheduleTimezone", "channelMessage"],
  webhook: ["channelMessage"],
  ag_ui: ["agUiGenericToolText"],
  fcp: ["fcpHandshake"],
} as const;

export function useChannelChatDraft({
  state,
  setState,
  id,
  baseline,
  writable,
  available = true,
}: {
  state: ChannelFormState;
  setState: Dispatch<SetStateAction<ChannelFormState>>;
  id?: string;
  baseline: string;
  writable: boolean;
  available?: boolean;
}) {
  const [initialState] = useState(state);
  const fields = fieldsByKind[state.kind as keyof typeof fieldsByKind] ?? [];
  const values = Object.fromEntries(fields.map((field) => [field, state[field]])) as Record<
    (typeof fields)[number],
    string
  >;
  return useTextPageFormDraft({
    fields,
    values,
    entityId: id,
    entityType: "agent_channel",
    formId: state.kind,
    label: `${state.kind.replaceAll("_", " ")} channel`,
    baseline,
    dirty: state !== initialState,
    writable,
    enabled: available && fields.length > 0,
    // Tokens, OAuth grants, Slack credentials and auth configuration never enter this adapter.
    apply: (next) => setState((previous) => ({ ...previous, ...next })),
  });
}
