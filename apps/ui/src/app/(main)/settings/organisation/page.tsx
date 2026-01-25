"use client";

import { useState, useEffect } from "react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { CopyButton } from "@/components/ui/copy-button";
import { useOrganization, useUpdateOrganization } from "@/hooks/use-organizations";
import { useOrg } from "@/providers/org-provider";
import { Building2, Save, AlertCircle } from "lucide-react";

export default function OrganisationPage() {
  const { currentOrg } = useOrg();
  const { data: organization, isLoading, error } = useOrganization();
  const updateOrganization = useUpdateOrganization();

  const [name, setName] = useState("");
  const [hasChanges, setHasChanges] = useState(false);

  // Sync name from fetched organization data
  useEffect(() => {
    if (organization?.name) {
      setName(organization.name);
      setHasChanges(false);
    }
  }, [organization?.name]);

  const handleNameChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setName(e.target.value);
    setHasChanges(e.target.value !== organization?.name);
  };

  const handleSave = async () => {
    if (!hasChanges || !name.trim()) return;

    await updateOrganization.mutateAsync({ name: name.trim() });
    setHasChanges(false);
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter" && hasChanges && name.trim()) {
      handleSave();
    }
  };

  if (error) {
    return (
      <div className="space-y-8">
        <section>
          <div className="mb-4">
            <h2 className="text-xl font-semibold">Organisation</h2>
            <p className="text-sm text-muted-foreground">
              Manage organisation settings.
            </p>
          </div>
          <Card className="p-8 text-center">
            <AlertCircle className="h-12 w-12 mx-auto text-destructive mb-4" />
            <h3 className="text-lg font-medium mb-2">Failed to load organisation</h3>
            <p className="text-muted-foreground">{error.message}</p>
          </Card>
        </section>
      </div>
    );
  }

  return (
    <div className="space-y-8">
      <section>
        <div className="mb-6">
          <h2 className="text-xl font-semibold">Organisation</h2>
          <p className="text-sm text-muted-foreground">
            Manage organisation settings.
          </p>
        </div>

        {isLoading ? (
          <Card className="p-6">
            <div className="space-y-6">
              <div className="space-y-2">
                <Skeleton className="h-4 w-24" />
                <Skeleton className="h-10 w-full max-w-md" />
              </div>
              <div className="space-y-2">
                <Skeleton className="h-4 w-32" />
                <Skeleton className="h-10 w-full max-w-md" />
              </div>
            </div>
          </Card>
        ) : (
          <Card className="p-6">
            <div className="flex items-center gap-3 mb-6">
              <div className="p-2 rounded-lg bg-primary/10">
                <Building2 className="h-5 w-5 text-primary" />
              </div>
              <div>
                <h3 className="font-medium">{organization?.name || currentOrg?.name}</h3>
                <p className="text-sm text-muted-foreground">Organisation settings</p>
              </div>
            </div>

            <div className="space-y-6 max-w-md">
              {/* Organisation Name */}
              <div className="space-y-2">
                <Label htmlFor="org-name">Organisation Name</Label>
                <div className="flex gap-2">
                  <Input
                    id="org-name"
                    value={name}
                    onChange={handleNameChange}
                    onKeyDown={handleKeyDown}
                    placeholder="Organisation name"
                    className="flex-1"
                  />
                  <Button
                    onClick={handleSave}
                    disabled={!hasChanges || !name.trim() || updateOrganization.isPending}
                  >
                    <Save className="h-4 w-4 mr-2" />
                    {updateOrganization.isPending ? "Saving..." : "Save"}
                  </Button>
                </div>
                {updateOrganization.isError && (
                  <p className="text-sm text-destructive">
                    Failed to update: {updateOrganization.error.message}
                  </p>
                )}
                {updateOrganization.isSuccess && !hasChanges && (
                  <p className="text-sm text-green-600">Saved successfully</p>
                )}
              </div>

              {/* Organisation ID */}
              <div className="space-y-2">
                <Label htmlFor="org-id">Organisation ID</Label>
                <div className="flex items-center gap-2">
                  <Input
                    id="org-id"
                    value={organization?.id || currentOrg?.public_id || ""}
                    readOnly
                    className="font-mono text-sm bg-muted"
                  />
                  <CopyButton value={organization?.id || currentOrg?.public_id || ""} />
                </div>
                <p className="text-xs text-muted-foreground">
                  Use this ID when making API calls scoped to this organisation.
                </p>
              </div>
            </div>
          </Card>
        )}
      </section>
    </div>
  );
}
