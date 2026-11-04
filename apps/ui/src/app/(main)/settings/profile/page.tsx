"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { useTextPageFormDraft } from "@/components/chat-mode/use-text-page-form-draft";

import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar";
import { useAuth } from "@/providers/auth-provider";
import { useUpdateProfile } from "@/hooks/use-auth";
import { usePageTitle } from "@/hooks";
import { LanguageSettings } from "@/components/settings/language-settings";
import { Save, User } from "lucide-react";

function getInitials(name: string): string {
  return name
    .split(" ")
    .map((n) => n[0])
    .join("")
    .toUpperCase()
    .slice(0, 2);
}

export default function ProfilePage() {
  const ui = useUiTranslations();
  usePageTitle(ui("Account"), ui("Settings"));
  const { user } = useAuth();
  const updateProfile = useUpdateProfile();

  const [nameOverride, setName] = useState<string | undefined>();
  const name = nameOverride ?? user?.name ?? "";
  const hasChanges = name !== (user?.name ?? "");
  const chatDraft = useTextPageFormDraft({
    fields: ["name"],
    values: { name },
    entityId: user?.id,
    entityType: "profile",
    label: ui("Account"),
    baseline: user?.name ?? "",
    dirty: hasChanges,
    writable: !!user && !updateProfile.isPending,
    apply: (values) => setName(values.name),
  });

  useEffect(() => {
    if (!chatDraft.adopted && user?.name) setName(undefined);
  }, [chatDraft.adopted, user?.name]);

  const handleNameChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setName(e.target.value);
  };

  const handleSave = async () => {
    if (chatDraft.blocked || !hasChanges || !name.trim()) return;
    await updateProfile.mutateAsync({ name: name.trim() });
    setName(undefined);
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter" && hasChanges && name.trim()) {
      handleSave();
    }
  };

  const displayName = name.trim() || user?.name || "";

  return (
    <div className="space-y-8">
      <LanguageSettings />
      <section>
        <div className="mb-6">
          <h2 className="text-xl font-semibold">{ui("Account")}</h2>
          <p className="text-sm text-muted-foreground">
            {ui("Your Everruns sign-in name and email.")}
          </p>
        </div>

        {chatDraft.notice}
        <Card className="p-6">
          <div className="flex items-center gap-3 mb-6">
            <Avatar className="h-12 w-12">
              {user?.avatar_url && (
                <AvatarImage src={user.avatar_url} alt={displayName || user?.email} />
              )}
              <AvatarFallback>
                {displayName ? getInitials(displayName) : <User className="h-5 w-5" />}
              </AvatarFallback>
            </Avatar>
            <div>
              <h3 className="font-medium">{displayName || user?.email || ui("User")}</h3>
              <p className="text-sm text-muted-foreground">{user?.email}</p>
            </div>
          </div>

          <div className="space-y-6 max-w-md">
            {/* Full Name */}
            <div className="space-y-2">
              <Label htmlFor="full-name">{ui("Full Name")}</Label>
              <div className="flex gap-2">
                <Input
                  id="full-name"
                  value={name}
                  onChange={handleNameChange}
                  onKeyDown={handleKeyDown}
                  placeholder={ui("Enter your full name")}
                  className="flex-1"
                />
                <Button
                  onClick={handleSave}
                  disabled={
                    chatDraft.blocked || !hasChanges || !name.trim() || updateProfile.isPending
                  }
                >
                  <Save className="h-4 w-4 mr-2" />
                  {updateProfile.isPending ? ui("Saving...") : ui("Save")}
                </Button>
                {hasChanges && chatDraft.adopted && (
                  <Button variant="outline" onClick={() => setName(undefined)}>
                    {ui("Discard")}
                  </Button>
                )}
              </div>
              {updateProfile.isError && (
                <p className="text-sm text-destructive">
                  {ui("Failed to update:")} {updateProfile.error.message}
                </p>
              )}
              {updateProfile.isSuccess && !hasChanges && (
                <p className="text-sm text-success">{ui("Saved successfully")}</p>
              )}
            </div>

            {/* Email (read-only) */}
            <div className="space-y-2">
              <Label htmlFor="email">{ui("Email")}</Label>
              <Input id="email" value={user?.email || ""} readOnly className="bg-muted" />
              <p className="text-xs text-muted-foreground">{ui("Email cannot be changed.")}</p>
            </div>
          </div>
        </Card>
      </section>
    </div>
  );
}
