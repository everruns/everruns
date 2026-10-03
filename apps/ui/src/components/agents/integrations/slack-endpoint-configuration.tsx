"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";
import {
  ChannelForm,
  buildChannelConfig,
  getDefaultChannelFormState,
  isChannelFormValid,
  type ChannelFormState,
} from "@/components/apps/channel-form";
import { useSlackInstallCapability, useUpdateAgentEndpoint } from "@/hooks/use-agent-endpoints";
import { SlackConnectionStatus } from "./slack-setup-guidance";
import {
  getSessionStrategyDisplayName,
  getSlackReplyModeDisplayName,
} from "@/lib/endpoint-display";
import type { AgentEndpoint, SlackChannelConfig } from "@/lib/api/types";

export function SlackEndpointConfiguration({
  agentId,
  endpoint,
  canManage,
}: {
  agentId: string;
  endpoint: AgentEndpoint;
  canManage: boolean;
}) {
  const capability = useSlackInstallCapability(canManage);
  const update = useUpdateAgentEndpoint(agentId, endpoint.id);
  const [draft, setDraft] = useState<ChannelFormState | null>(null);
  const [saved, setSaved] = useState(false);
  // Workspace choice belongs to installation and survives saving the endpoint settings.
  const [installTeamId, setInstallTeamId] = useState("");
  const initial = getDefaultChannelFormState("slack", endpoint);
  const state = { ...(draft ?? initial), slackInstallTeamId: installTeamId };
  const dirty =
    state.enabled !== initial.enabled ||
    JSON.stringify(buildChannelConfig(state)) !== JSON.stringify(buildChannelConfig(initial));
  const config = endpoint.channel_config as SlackChannelConfig;

  if (!canManage) {
    return (
      <div className="space-y-4">
        <SlackConnectionStatus endpoint={endpoint} />
        <p className="text-sm text-muted-foreground">
          {getSessionStrategyDisplayName(config.session_strategy ?? "per_thread")} ·{" "}
          {getSlackReplyModeDisplayName(config.reply_mode ?? "all_messages")}
        </p>
      </div>
    );
  }

  return (
    <form
      className="space-y-4"
      aria-label="Slack endpoint configuration"
      onSubmit={(event) => {
        event.preventDefault();
        if (!dirty || update.isPending || !isChannelFormValid(state)) return;
        setSaved(false);
        update.mutate(
          { channel_config: buildChannelConfig(state), enabled: state.enabled },
          {
            onSuccess: () => {
              setDraft(null);
              setSaved(true);
            },
          },
        );
      }}
    >
      <fieldset disabled={update.isPending} className="min-w-0 space-y-4">
        <ChannelForm
          state={state}
          onChange={(next) => {
            if (update.isPending) return;
            const settingsChanged =
              next.enabled !== state.enabled ||
              JSON.stringify(buildChannelConfig(next)) !==
                JSON.stringify(buildChannelConfig(state));
            setInstallTeamId(next.slackInstallTeamId);
            if (settingsChanged) {
              setDraft(next);
              setSaved(false);
              update.reset();
            }
          }}
          mode="edit"
          endpointId={endpoint.id}
          endpoint={endpoint}
          slackInstallDisabled={dirty}
          slackInstallCapability={capability.data}
          onSlackCapabilityChanged={capability.refetch}
        />
      </fieldset>
      {update.isError && (
        <p role="alert" className="text-sm text-destructive">
          {update.error.message}
        </p>
      )}
      <div className="flex flex-wrap items-center justify-end gap-3">
        <span role="status" className="text-xs text-muted-foreground">
          {saved ? "Changes saved" : dirty ? "Unsaved changes" : ""}
        </span>
        {dirty && (
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={update.isPending}
            onClick={() => {
              setDraft(null);
              update.reset();
            }}
          >
            Discard changes
          </Button>
        )}
        <Button
          type="submit"
          size="sm"
          disabled={!dirty || update.isPending || !isChannelFormValid(state)}
        >
          {update.isPending ? "Saving…" : "Save changes"}
        </Button>
      </div>
    </form>
  );
}
