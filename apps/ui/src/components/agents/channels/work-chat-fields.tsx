"use client";
import { useId } from "react";
import type { ChannelFormState } from "./channel-form";
import { FormSection } from "./form-section";
import {
  Select as SharedSelect,
  SelectTrigger as SharedSelectTrigger,
  SelectContent as SharedSelectContent,
  SelectValue as SharedSelectValue,
  SelectItem as SharedSelectItem,
} from "@/components/ui/select";
import { useUiTranslations } from "@/providers/locale-provider";
import { useCurrentUser } from "@/hooks/use-auth";
import { useParams } from "next/navigation";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api/client";
import { ApiFields, type ApiFormState } from "./api-fields";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";

export type WorkChatMembership = "open" | "invites" | "org_users";

const MAX_INVITES = 50;

export function plausibleEmail(value: string): boolean {
  const email = value.trim();
  const at = email.lastIndexOf("@");
  if (at <= 0 || email.indexOf("@") !== at) return false;
  const domain = email.slice(at + 1);
  return domain.includes(".") && !/\s/.test(email) && domain.length > 1;
}

export function inviteList(value: string): string[] {
  const seen = new Set<string>();
  const emails: string[] = [];
  for (const line of value.split("\n")) {
    const email = line.trim().toLowerCase();
    if (!email || seen.has(email)) continue;
    seen.add(email);
    emails.push(email);
  }
  return emails;
}

export function isWorkChatMembershipValid(
  membership: WorkChatMembership,
  memberEmail: string,
  invites: string,
): boolean {
  if (membership !== "invites") return true;
  const people = inviteList(invites);
  return (
    plausibleEmail(memberEmail) &&
    people.length <= MAX_INVITES &&
    people.every((email) => plausibleEmail(email))
  );
}

function WorkChatFields({
  mode,
  onModeChange,
  membership,
  memberEmail,
  invites,
  onMembershipChange,
  value,
  onChange,
}: {
  mode: "single_with_threads" | "multi_chat";
  onModeChange: (mode: "single_with_threads" | "multi_chat") => void;
  membership: WorkChatMembership;
  memberEmail: string;
  invites: string;
  onMembershipChange: (next: {
    membership: WorkChatMembership;
    memberEmail: string;
    invites: string;
  }) => void;
  value: ApiFormState;
  onChange: (value: ApiFormState) => void;
}) {
  const ui = useUiTranslations();
  const user = useCurrentUser();
  const params = useParams();
  const agentId = String(params.agentId ?? "");
  const features = useQuery({
    queryKey: ["chat-features", agentId],
    enabled: !!agentId,
    queryFn: async () =>
      (
        await api.get<{ threads: boolean; files: boolean }>(
          `/v1/platform/agents/${encodeURIComponent(agentId)}/chat-features`,
        )
      ).data,
  });
  return (
    <div className="space-y-4">
      <Label htmlFor="chat-interface">{ui("Interface")}</Label>
      <SharedSelect
        value={String(mode)}
        onValueChange={(selection) => onModeChange(selection as typeof mode)}
      >
        <SharedSelectTrigger id="chat-interface" className="w-full border bg-background p-2">
          <SharedSelectValue />
        </SharedSelectTrigger>
        <SharedSelectContent>
          <SharedSelectItem value="multi_chat">{ui("Multiple conversations")}</SharedSelectItem>
          <SharedSelectItem value="single_with_threads" disabled={!features.data?.threads}>
            {ui("Single chat with threads")}
          </SharedSelectItem>
        </SharedSelectContent>
      </SharedSelect>
      {!features.data?.threads && (
        <p className="text-sm text-muted-foreground">
          {ui(
            "Single chat with threads requires the Agent’s coordinator capability and thread tools.",
          )}
        </p>
      )}
      <div className="space-y-2">
        <Label htmlFor="chat-membership">{ui("Who can use this chat")}</Label>
        <SharedSelect
          value={membership}
          onValueChange={(selection) => {
            const next = selection as WorkChatMembership;
            onMembershipChange({
              membership: next,
              memberEmail:
                next === "invites" && !memberEmail.trim() && user.data?.email
                  ? user.data.email
                  : memberEmail,
              invites,
            });
          }}
        >
          <SharedSelectTrigger id="chat-membership" className="w-full border bg-background p-2">
            <SharedSelectValue />
          </SharedSelectTrigger>
          <SharedSelectContent>
            <SharedSelectItem value="open">{ui("Open access")}</SharedSelectItem>
            <SharedSelectItem value="invites">{ui("Single user + invites")}</SharedSelectItem>
            <SharedSelectItem value="org_users">{ui("Organization users")}</SharedSelectItem>
          </SharedSelectContent>
        </SharedSelect>
        <p className="text-xs text-muted-foreground">
          {ui(
            membership === "invites"
              ? "Only one person, plus the people they invite."
              : membership === "org_users"
                ? "Members of this organization can sign in."
                : "Anyone can register and sign in.",
          )}
        </p>
      </div>
      {membership === "invites" && (
        <div className="grid gap-4 md:grid-cols-2">
          <div className="space-y-2">
            <Label htmlFor="chat-member-email">{ui("This person")}</Label>
            <Input
              id="chat-member-email"
              type="email"
              value={memberEmail}
              onChange={(event) =>
                onMembershipChange({ membership, memberEmail: event.target.value, invites })
              }
              placeholder="ada@example.com"
              aria-invalid={!plausibleEmail(memberEmail)}
            />
            <p className="text-xs text-muted-foreground">{ui("They sign in with this email.")}</p>
            {!plausibleEmail(memberEmail) && (
              <p className="text-xs text-destructive">{ui("Enter this person's email.")}</p>
            )}
          </div>
          <div className="space-y-2">
            <Label htmlFor="chat-invites">{ui("Invites")}</Label>
            <Textarea
              id="chat-invites"
              value={invites}
              onChange={(event) =>
                onMembershipChange({ membership, memberEmail, invites: event.target.value })
              }
              placeholder={"ada@example.com\ngrace@example.com"}
              rows={3}
              aria-invalid={
                !inviteList(invites).every((email) => plausibleEmail(email)) ||
                inviteList(invites).length > MAX_INVITES
              }
            />
            <p className="text-xs text-muted-foreground">
              {ui("One email per line. Each person signs in with their own account.")}
            </p>
            {!inviteList(invites).every((email) => plausibleEmail(email)) ||
            inviteList(invites).length > MAX_INVITES ? (
              <p className="text-xs text-destructive">
                {ui("Use email addresses, one per line, up to 50.")}
              </p>
            ) : null}
          </div>
        </div>
      )}
      <p className="text-sm text-muted-foreground">
        {ui(
          "Consumers sign in separately from the console. Your configured approvals and spending limits apply.",
        )}
      </p>
      <p className="text-sm text-muted-foreground">
        {ui("Messages use the model configured on the Agent.")}
      </p>
      <ApiFields
        hosted
        value={value}
        onChange={(next) => onChange({ ...next, errors: "public" })}
      />
    </div>
  );
}

export function WorkChatSection({
  state,
  onChange,
}: {
  state: ChannelFormState;
  onChange: (state: ChannelFormState) => void;
}) {
  const ui = useUiTranslations();
  const placementId = useId();
  return (
    <FormSection title="Chat site" hint="How conversations are organized.">
      <WorkChatFields
        mode={state.chatInterface}
        onModeChange={(mode) => onChange({ ...state, chatInterface: mode })}
        membership={state.chatMembership}
        memberEmail={state.chatMemberEmail}
        invites={state.chatInvites}
        onMembershipChange={(next) =>
          onChange({
            ...state,
            chatMembership: next.membership,
            chatMemberEmail: next.memberEmail,
            chatInvites: next.invites,
          })
        }
        value={state.api}
        onChange={(api) => onChange({ ...state, api })}
      />
      {
        <div className="space-y-2">
          <Label htmlFor={placementId}>{ui("Published view")}</Label>
          <select
            id={placementId}
            className="h-9 w-full border bg-background px-2 text-sm"
            value={state.chatViewPlacement}
            onChange={(event) => {
              const value = event.target.value;
              onChange({
                ...state,
                chatViewPlacement: value === "page" || value === "panel" ? value : "none",
              });
            }}
          >
            <option value="none">{ui("None")}</option>
            <option value="panel">{ui("Show alongside chat")}</option>
            <option value="page">{ui("Open on the published page")}</option>
          </select>
          <p className="text-xs text-muted-foreground">
            {ui("This shares the Agent’s view with people who can sign into this channel.")}
          </p>
        </div>
      }
      <WorkChatPluginOptions
        value={state.chatPlugins}
        onChange={(chatPlugins) => onChange({ ...state, chatPlugins })}
      />
    </FormSection>
  );
}

/** What each person may change about the plugins in their own conversations.
 * Both off keeps every conversation on exactly the agent's plugins. */
function WorkChatPluginOptions({
  value,
  onChange,
}: {
  value: ChannelFormState["chatPlugins"];
  onChange: (value: ChannelFormState["chatPlugins"]) => void;
}) {
  const ui = useUiTranslations();
  const addingId = useId();
  const turningOffId = useId();
  return (
    <div className="space-y-3">
      <div>
        <Label>{ui("Plugins")}</Label>
        <p className="mt-1 text-xs text-muted-foreground">
          {ui(
            "People manage these in Customizations in the chat. Changes apply to their new conversations.",
          )}
        </p>
      </div>
      <div className="flex items-start justify-between gap-4">
        <div className="space-y-0.5">
          <Label htmlFor={addingId}>{ui("Let people add plugins")}</Label>
          <p className="text-xs text-muted-foreground">
            {ui("Any plugin your organization approved, for their own conversations.")}
          </p>
        </div>
        <Switch
          id={addingId}
          checked={value.allow_adding}
          onCheckedChange={(allow_adding) => onChange({ ...value, allow_adding })}
        />
      </div>
      <div className="flex items-start justify-between gap-4">
        <div className="space-y-0.5">
          <Label htmlFor={turningOffId}>{ui("Let people turn off the agent's plugins")}</Label>
          <p className="text-xs text-muted-foreground">
            {ui("Each person can switch off a plugin the agent includes.")}
          </p>
        </div>
        <Switch
          id={turningOffId}
          checked={value.allow_turning_off}
          onCheckedChange={(allow_turning_off) => onChange({ ...value, allow_turning_off })}
        />
      </div>
    </div>
  );
}
