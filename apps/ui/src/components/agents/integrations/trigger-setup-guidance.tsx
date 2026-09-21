import type { AgentTrigger, InvocationSessionMode } from "@/lib/api/types";
import { ScheduleSetupGuidance } from "@/components/agents/integrations/schedule-setup-guidance";
import { WebhookSetupGuidance } from "@/components/agents/integrations/webhook-setup-guidance";

export interface ScheduleTriggerConfig {
  cron_expression: string;
  timezone?: string;
  session_mode?: InvocationSessionMode;
  message: string;
}

export interface WebhookTriggerConfig {
  token?: string;
  token_configured?: boolean;
  session_mode?: InvocationSessionMode;
  message: string;
}

export function getScheduleTriggerConfig(trigger: AgentTrigger): ScheduleTriggerConfig {
  return trigger.config as ScheduleTriggerConfig;
}

export function getWebhookTriggerConfig(trigger: AgentTrigger): WebhookTriggerConfig {
  return trigger.config as WebhookTriggerConfig;
}

function webhookUrl(trigger: AgentTrigger): string | null {
  if (!trigger.ingress_id) return null;
  const origin = typeof window === "undefined" ? "" : window.location.origin;
  return `${origin}/api/v1/e/${trigger.ingress_id}/webhook`;
}

export function TriggerSetupGuidance({ trigger }: { trigger: AgentTrigger }) {
  if (trigger.trigger_type === "webhook") {
    const config = getWebhookTriggerConfig(trigger);
    const url = webhookUrl(trigger);
    if (!url) {
      return <p className="text-sm text-destructive">This webhook has no ingress URL.</p>;
    }
    return (
      <WebhookSetupGuidance
        endpointUrl={url}
        sessionMode={config.session_mode ?? "shared_session"}
        message={config.message}
        tokenConfigured={!!(config.token_configured || config.token)}
        isEnabled={trigger.enabled}
      />
    );
  }

  const config = getScheduleTriggerConfig(trigger);
  return (
    <ScheduleSetupGuidance
      cronExpression={config.cron_expression}
      timezone={config.timezone ?? "UTC"}
      sessionMode={config.session_mode ?? "shared_session"}
      message={config.message}
      isEnabled={trigger.enabled}
    />
  );
}
