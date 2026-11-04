"use client";

import { useMemo, useState, type Dispatch, type SetStateAction } from "react";
import { z } from "zod";
import type { ChannelFormState } from "@/components/agents/channels/channel-form";
import { draftText, usePageFormDraft } from "./use-page-form-draft";

const common = { enabled: z.boolean() };
const apiMetadata = z.strictObject({
  toolActivityText: draftText,
  rateLimitPerMinute: draftText,
  sessionBinding: z.enum(["per_user", "session_per_invocation"]),
});
const schemas = {
  public_chat: z.strictObject({
    ...common,
    publicChatDisplayName: draftText,
    publicChatWelcomeMessage: draftText,
    publicChatGenericToolText: draftText,
    publicChatPrimaryColor: draftText,
    publicChatRateLimitPerMinute: draftText,
  }),
  schedule: z.strictObject({
    ...common,
    scheduleCronExpression: draftText,
    scheduleTimezone: draftText,
    channelMessage: draftText,
    invocationSessionMode: z.enum(["shared_session", "session_per_invocation"]),
  }),
  webhook: z.strictObject({
    ...common,
    channelMessage: draftText,
    invocationSessionMode: z.enum(["shared_session", "session_per_invocation"]),
  }),
  ag_ui: z.strictObject({
    ...common,
    agUiGenericToolText: draftText,
    agUiRateLimitPerMinute: draftText,
  }),
  fcp: z.strictObject({
    ...common,
    fcpHandshake: draftText,
    fcpRateLimitPerMinute: draftText,
    fcpResponseTimeoutSeconds: draftText,
  }),
  slack: z.strictObject({
    ...common,
    slackSessionStrategy: z.enum(["per_thread", "per_channel", "per_user"]),
    slackResponsePolicy: z.enum(["all_messages", "mentions_only", "relevant_messages"]),
  }),
  voice: z.strictObject({
    ...common,
    voice: z.strictObject({
      model: draftText,
      voice: draftText,
      language: draftText,
      greeting: draftText,
      filler: draftText,
      fillerAfterMs: draftText,
      speakingStyle: draftText,
      turnDetection: z.enum(["server_vad", "semantic_vad"]),
      interruption: z.enum(["steer", "cancel"]),
    }),
  }),
  api: z.strictObject({ ...common, api: apiMetadata }),
  work_chat: z.strictObject({
    ...common,
    api: apiMetadata,
    chatInterface: z.enum(["single_with_threads", "multi_chat"]),
    chatMembership: z.enum(["open", "invites", "org_users"]),
    chatMemberEmail: draftText,
    chatInvites: draftText,
  }),
};

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
  const schema = useMemo(
    () => schemas[state.kind as keyof typeof schemas] ?? z.strictObject(common),
    [state.kind],
  );
  // Parse an allowlisted projection. In particular, nested API auth methods,
  // browser sign-in config, channel tokens and Slack credentials are excluded.
  const values = Object.fromEntries(
    Object.keys(schema.shape).map((field) => [
      field,
      field === "api"
        ? {
            toolActivityText: state.api.toolActivityText,
            rateLimitPerMinute: state.api.rateLimitPerMinute,
            sessionBinding: state.api.sessionBinding,
          }
        : state[field as keyof ChannelFormState],
    ]),
  );
  return usePageFormDraft<Record<string, unknown>>({
    schema: schema as z.ZodType<Record<string, unknown>>,
    values,
    entityId: id,
    entityType: "agent_channel",
    formId: state.kind,
    label: `${state.kind.replaceAll("_", " ")} channel`,
    baseline,
    dirty: state !== initialState,
    writable,
    enabled: available,
    apply: (next) =>
      setState(
        (previous) =>
          ({
            ...previous,
            ...next,
            ...(next.api
              ? { api: { ...previous.api, ...(next.api as Partial<ChannelFormState["api"]>) } }
              : {}),
          }) as ChannelFormState,
      ),
  });
}
