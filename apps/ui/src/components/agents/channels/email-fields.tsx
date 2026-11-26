"use client";
import { useQuery } from "@tanstack/react-query";
import Link from "next/link";
import { api } from "@/lib/api/client";
import { useOrg } from "@/providers/org-provider";
import { useUiTranslations } from "@/providers/locale-provider";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Switch } from "@/components/ui/switch";
import { FormSection } from "./form-section";
export type EmailConfig = {
  inbox_id: string;
  purpose: string;
  auto_dispatch: boolean;
  auto_reply: boolean;
  reply_template_id?: string | null;
};
export function EmailFields({
  value,
  onChange,
}: {
  value: EmailConfig;
  onChange: (value: EmailConfig) => void;
}) {
  const ui = useUiTranslations();
  const { currentOrg, hasRole } = useOrg();
  const admin = hasRole("admin");
  const inboxes = useQuery({
    queryKey: ["email", currentOrg?.public_id, "inboxes"],
    enabled: admin && !!currentOrg,
    queryFn: async () =>
      (await api.get<{ id: string; name: string; address: string }[]>("/v1/platform/email/inboxes"))
        .data,
  });
  const templates = useQuery({
    queryKey: ["email", currentOrg?.public_id, "templates"],
    enabled: admin && !!currentOrg,
    queryFn: async () =>
      (await api.get<{ id: string; name: string }[]>("/v1/platform/email/templates")).data,
  });
  return (
    <FormSection
      title={ui("Email inbox")}
      hint={ui("One inbox connects to one agent. Each correspondent has a separate conversation.")}
    >
      {!admin && <p>{ui("Organization admin access required")}</p>}
      <Label htmlFor="channel-email-inbox">{ui("Inbox")}</Label>
      <select
        id="channel-email-inbox"
        disabled={!admin}
        value={value.inbox_id}
        onChange={(e) => onChange({ ...value, inbox_id: e.target.value })}
        className="w-full border border-border bg-background p-2"
      >
        <option value="">{ui("Select inbox")}</option>
        {inboxes.data?.map((i) => (
          <option key={i.id} value={i.id}>
            {i.name} · {i.address}
          </option>
        ))}
      </select>
      <Label htmlFor="channel-email-template">{ui("Reply template")}</Label>
      <select
        id="channel-email-template"
        className="w-full border border-border bg-background p-2"
        value={value.reply_template_id ?? ""}
        onChange={(e) => onChange({ ...value, reply_template_id: e.target.value || null })}
      >
        <option value="">{ui("Automatic text and HTML")}</option>
        {templates.data?.map((t) => (
          <option key={t.id} value={t.id}>
            {t.name}
          </option>
        ))}
      </select>
      <Link href="/email" className="text-sm underline">
        {ui("Manage inboxes")}
      </Link>
      <Label htmlFor="channel-email-purpose">{ui("What should this inbox answer?")}</Label>
      <Textarea
        id="channel-email-purpose"
        maxLength={4096}
        disabled={!admin}
        value={value.purpose}
        onChange={(e) => onChange({ ...value, purpose: e.target.value })}
      />
      <div className="flex items-center justify-between gap-4">
        <Label htmlFor="email-auto-run">
          {ui("Automatically run the agent after admission checks")}
        </Label>
        <Switch
          id="email-auto-run"
          disabled={!admin}
          checked={value.auto_dispatch}
          onCheckedChange={(auto_dispatch) => onChange({ ...value, auto_dispatch })}
        />
      </div>
      <div className="flex items-center justify-between gap-4">
        <Label htmlFor="email-auto-reply">
          {ui("Automatically send replies after output checks")}
        </Label>
        <Switch
          id="email-auto-reply"
          disabled={!admin}
          checked={value.auto_reply}
          onCheckedChange={(auto_reply) => onChange({ ...value, auto_reply })}
        />
      </div>
      <p className="text-sm text-muted-foreground">
        {ui("Uncertain messages and replies remain held for review.")}
      </p>
    </FormSection>
  );
}
