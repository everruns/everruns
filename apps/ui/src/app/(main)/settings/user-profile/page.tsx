"use client";

import { useState } from "react";
import { z } from "zod";
import { usePageFormDraft } from "@/components/chat-mode/use-page-form-draft";
import Link from "next/link";
import { useVirtualUser, useUpdateVirtualUser } from "@/hooks/use-virtual-users";
import { usePageTitle } from "@/hooks";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { Combobox } from "@/components/ui/combobox";
import { LOCALE_OPTIONS, TIMEZONE_OPTIONS } from "@/lib/locale-data";
import type { UpdateVirtualUserRequest } from "@/lib/api/types";
const profileSchema = z.strictObject({
  name: z.string(),
  locale: z.string(),
  timezone: z.string(),
});
export default function UserProfilePage() {
  usePageTitle("Profile", "Settings");
  const profile = useVirtualUser("me");
  const update = useUpdateVirtualUser();
  const [changes, setChanges] = useState<UpdateVirtualUserRequest>({});
  const saved = {
    name: profile.data?.name ?? "",
    locale: profile.data?.locale ?? "",
    timezone: profile.data?.timezone ?? "",
  };
  const values = {
    name: changes.name ?? saved.name,
    locale: changes.locale === undefined ? saved.locale : (changes.locale ?? ""),
    timezone: changes.timezone === undefined ? saved.timezone : (changes.timezone ?? ""),
  };
  const dirty = JSON.stringify(values) !== JSON.stringify(saved);
  const chat = usePageFormDraft({
    schema: profileSchema,
    entityType: "virtual_user",
    entityId: profile.data?.id,
    label: "Profile",
    enabled: !!profile.data,
    values,
    baseline: JSON.stringify(saved),
    dirty,
    writable: !!profile.data && !update.isPending,
    apply: (next) =>
      setChanges({
        ...changes,
        name: next.name,
        locale: next.locale || null,
        timezone: next.timezone || null,
      }),
  });
  if (profile.isLoading) return <p>Loading your profile…</p>;
  if (!profile.data) return <p className="text-destructive">Your profile could not be loaded.</p>;

  const user = profile.data;
  return (
    <div className="space-y-8">
      <div>
        <h2 className="text-xl font-semibold">Profile</h2>
        <p className="text-sm text-muted-foreground">
          Your name, locale, and timezone for chats in this organization.
        </p>
        <Link className="text-sm text-primary underline" href={`/virtual-users/${user.id}`}>
          View virtual user
        </Link>
      </div>
      <form
        className="space-y-4"
        onSubmit={async (e) => {
          e.preventDefault();
          if (chat.blocked) return;
          await update.mutateAsync({ identityId: "me", request: changes });
          await profile.refetch();
          setChanges({});
        }}
      >
        {chat.notice}
        <div className="space-y-2">
          <Label htmlFor="runtime-name">Name</Label>
          <Input
            disabled={chat.adopted && update.isPending}
            id="runtime-name"
            required
            value={changes.name ?? user.name}
            onChange={(e) => setChanges({ ...changes, name: e.target.value })}
          />
        </div>
        <div className="grid gap-4 md:grid-cols-2">
          <div className="space-y-2">
            <Label>Locale</Label>
            <Combobox
              disabled={chat.adopted && update.isPending}
              options={LOCALE_OPTIONS}
              value={changes.locale === undefined ? (user.locale ?? "") : (changes.locale ?? "")}
              onValueChange={(value) => setChanges({ ...changes, locale: value || null })}
            />
          </div>
          <div className="space-y-2">
            <Label>Timezone</Label>
            <Combobox
              disabled={chat.adopted && update.isPending}
              options={TIMEZONE_OPTIONS}
              value={
                changes.timezone === undefined ? (user.timezone ?? "") : (changes.timezone ?? "")
              }
              onValueChange={(value) => setChanges({ ...changes, timezone: value || null })}
            />
          </div>
        </div>
        <Button disabled={update.isPending || chat.blocked} type="submit">
          Save profile
        </Button>
        {chat.adopted && dirty && (
          <Button
            type="button"
            variant="outline"
            disabled={update.isPending}
            onClick={() => setChanges({})}
          >
            Discard
          </Button>
        )}
        {update.error && <p className="text-destructive">{update.error.message}</p>}
      </form>
    </div>
  );
}
