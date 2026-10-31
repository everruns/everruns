"use client";
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

export function WorkChatFields({
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
          `/v1/agents/${encodeURIComponent(agentId)}/chat-features`,
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
