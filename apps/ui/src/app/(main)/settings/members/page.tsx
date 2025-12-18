"use client";

import { Card } from "@/components/ui/card";
import { useAuth } from "@/providers/auth-provider";
import { Users, ShieldAlert, Construction } from "lucide-react";

export default function MembersPage() {
  const { requiresAuth } = useAuth();

  // If auth is not required, show a message
  if (!requiresAuth) {
    return (
      <div className="space-y-8">
        <section>
          <div className="mb-4">
            <h2 className="text-xl font-semibold">Members</h2>
            <p className="text-sm text-muted-foreground">
              Manage team members and access control.
            </p>
          </div>
          <Card className="p-8 text-center">
            <ShieldAlert className="h-12 w-12 mx-auto text-muted-foreground mb-4" />
            <h3 className="text-lg font-medium mb-2">Authentication Disabled</h3>
            <p className="text-muted-foreground">
              Member management is only available when authentication is enabled.
              Contact your administrator to enable authentication.
            </p>
          </Card>
        </section>
      </div>
    );
  }

  return (
    <div className="space-y-8">
      <section>
        <div className="flex items-center justify-between mb-4">
          <div>
            <h2 className="text-xl font-semibold">Members</h2>
            <p className="text-sm text-muted-foreground">
              Manage team members and access control.
            </p>
          </div>
        </div>

        <Card className="p-8 text-center">
          <Construction className="h-12 w-12 mx-auto text-muted-foreground mb-4" />
          <h3 className="text-lg font-medium mb-2">Coming Soon</h3>
          <p className="text-muted-foreground mb-4">
            Member management functionality is under development.
            This feature will allow you to invite team members, manage roles, and control access.
          </p>
          <div className="flex justify-center">
            <Users className="h-6 w-6 text-muted-foreground" />
          </div>
        </Card>
      </section>
    </div>
  );
}
