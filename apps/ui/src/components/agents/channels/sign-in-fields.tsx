"use client";

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";

type SignInField =
  | "publicChatGoogleEnabled"
  | "publicChatGoogleClientId"
  | "publicChatGoogleAllowedDomains"
  | "publicChatAgentIdEnabled"
  | "publicChatAgentIdClientId";

type SignInUpdate = {
  (key: "publicChatGoogleEnabled" | "publicChatAgentIdEnabled", value: boolean): void;
  (
    key: Exclude<SignInField, "publicChatGoogleEnabled" | "publicChatAgentIdEnabled">,
    value: string,
  ): void;
};

/** Public Chat visitor sign-in: Google or AgentID, one at a time. */
export function PublicChatSignInFields({
  googleEnabled,
  googleClientId,
  googleAllowedDomains,
  agentIdEnabled,
  agentIdClientId,
  update,
}: {
  googleEnabled: boolean;
  googleClientId: string;
  googleAllowedDomains: string;
  agentIdEnabled: boolean;
  agentIdClientId: string;
  update: SignInUpdate;
}) {
  return (
    <>
      <div className="space-y-3">
        <div className="flex items-center justify-between border p-3">
          <div>
            <p className="text-sm font-medium">Google sign-in</p>
            <p className="text-xs text-muted-foreground">
              Require visitors to sign in with Google. When on, every request must present a valid
              Google account; signed-in visitors skip the Turnstile challenge.
            </p>
          </div>
          <Switch
            checked={googleEnabled}
            onCheckedChange={(checked) => {
              update("publicChatGoogleEnabled", checked);
              if (checked) update("publicChatAgentIdEnabled", false);
            }}
          />
        </div>
        {googleEnabled && (
          <div className="grid gap-4 md:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="public_chat_google_client_id">Google OAuth client ID</Label>
              <Input
                id="public_chat_google_client_id"
                value={googleClientId}
                onChange={(event) => update("publicChatGoogleClientId", event.target.value)}
                className="font-mono"
                placeholder="1234-abc.apps.googleusercontent.com"
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="public_chat_google_domains">Allowed domains (optional)</Label>
              <Input
                id="public_chat_google_domains"
                value={googleAllowedDomains}
                onChange={(event) => update("publicChatGoogleAllowedDomains", event.target.value)}
                placeholder="example.com, partner.com"
              />
              <p className="text-xs text-muted-foreground">
                Comma-separated. Leave blank to allow any Google account.
              </p>
            </div>
          </div>
        )}
      </div>
      <AgentIdSignInFields
        idPrefix="public_chat"
        description="Let AI agents sign in with their AgentID. Every request must present a valid AgentID token for your client; signed-in agents skip the Turnstile challenge."
        enabled={agentIdEnabled}
        clientId={agentIdClientId}
        onEnabledChange={(checked) => {
          update("publicChatAgentIdEnabled", checked);
          if (checked) update("publicChatGoogleEnabled", false);
        }}
        onClientIdChange={(value) => update("publicChatAgentIdClientId", value)}
      />
    </>
  );
}

/** Switch plus client ID field for the AgentID channel preset. */
export function AgentIdSignInFields({
  idPrefix,
  description,
  enabled,
  clientId,
  onEnabledChange,
  onClientIdChange,
}: {
  idPrefix: string;
  description: string;
  enabled: boolean;
  clientId: string;
  onEnabledChange: (checked: boolean) => void;
  onClientIdChange: (value: string) => void;
}) {
  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between border p-3">
        <div>
          <p className="text-sm font-medium">AgentID sign-in</p>
          <p className="text-xs text-muted-foreground">{description}</p>
        </div>
        <Switch checked={enabled} onCheckedChange={onEnabledChange} aria-label="AgentID sign-in" />
      </div>
      {enabled && (
        <div className="space-y-2">
          <Label htmlFor={`${idPrefix}_agentid_client_id`}>AgentID client ID</Label>
          <Input
            id={`${idPrefix}_agentid_client_id`}
            value={clientId}
            onChange={(event) => onClientIdChange(event.target.value)}
            className="font-mono"
            placeholder="b7d41e0a-2c65-4f8b-9d31-0a5e7c2f4b18"
          />
          <p className="text-xs text-muted-foreground">
            The client ID of your app in the AgentID console. Tokens must be issued for it and last
            ten minutes; AgentID is a sign-in, not a long-lived API key.
          </p>
        </div>
      )}
    </div>
  );
}
