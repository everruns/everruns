"use client";

import { useState, useEffect } from "react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar";
import { useAuth } from "@/providers/auth-provider";
import { useUpdateProfile } from "@/hooks/use-auth";
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
  const { user } = useAuth();
  const updateProfile = useUpdateProfile();

  const [name, setName] = useState("");
  const [hasChanges, setHasChanges] = useState(false);

  useEffect(() => {
    if (user?.name) {
      setName(user.name);
      setHasChanges(false);
    }
  }, [user?.name]);

  const handleNameChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setName(e.target.value);
    setHasChanges(e.target.value !== (user?.name ?? ""));
  };

  const handleSave = async () => {
    if (!hasChanges || !name.trim()) return;
    await updateProfile.mutateAsync({ name: name.trim() });
    setHasChanges(false);
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter" && hasChanges && name.trim()) {
      handleSave();
    }
  };

  const displayName = name.trim() || user?.name || "";

  return (
    <div className="space-y-8">
      <section>
        <div className="mb-6">
          <h2 className="text-xl font-semibold">Profile</h2>
          <p className="text-sm text-muted-foreground">Manage your personal profile.</p>
        </div>

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
              <h3 className="font-medium">{displayName || user?.email || "User"}</h3>
              <p className="text-sm text-muted-foreground">{user?.email}</p>
            </div>
          </div>

          <div className="space-y-6 max-w-md">
            {/* Full Name */}
            <div className="space-y-2">
              <Label htmlFor="full-name">Full Name</Label>
              <div className="flex gap-2">
                <Input
                  id="full-name"
                  value={name}
                  onChange={handleNameChange}
                  onKeyDown={handleKeyDown}
                  placeholder="Enter your full name"
                  className="flex-1"
                />
                <Button
                  onClick={handleSave}
                  disabled={!hasChanges || !name.trim() || updateProfile.isPending}
                >
                  <Save className="h-4 w-4 mr-2" />
                  {updateProfile.isPending ? "Saving..." : "Save"}
                </Button>
              </div>
              {updateProfile.isError && (
                <p className="text-sm text-destructive">
                  Failed to update: {updateProfile.error.message}
                </p>
              )}
              {updateProfile.isSuccess && !hasChanges && (
                <p className="text-sm text-green-600">Saved successfully</p>
              )}
            </div>

            {/* Email (read-only) */}
            <div className="space-y-2">
              <Label htmlFor="email">Email</Label>
              <Input id="email" value={user?.email || ""} readOnly className="bg-muted" />
              <p className="text-xs text-muted-foreground">Email cannot be changed.</p>
            </div>
          </div>
        </Card>
      </section>
    </div>
  );
}
