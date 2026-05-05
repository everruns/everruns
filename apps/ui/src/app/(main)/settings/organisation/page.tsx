"use client";

import { useState, useEffect } from "react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { CopyButton } from "@/components/ui/copy-button";
import { HarnessSelect } from "@/components/harness/harness-select";
import { useOrganization, useUpdateOrganization } from "@/hooks/use-organizations";
import { useHarnesses, usePageTitle } from "@/hooks";
import { useOrg } from "@/providers/org-provider";
import { Building2, Save, AlertCircle } from "lucide-react";

export default function OrganisationPage() {
  usePageTitle("Organisation", "Settings");
  const { currentOrg } = useOrg();
  const { data: organization, isLoading, error } = useOrganization();
  const { data: harnesses = [] } = useHarnesses();
  const updateOrganization = useUpdateOrganization();

  const [name, setName] = useState("");
  const [defaultHarnessId, setDefaultHarnessId] = useState("");
  const [baseHarnessId, setBaseHarnessId] = useState("");
  const [hasChanges, setHasChanges] = useState(false);

  const syncHasChanges = (
    nextName: string,
    nextDefaultHarnessId: string,
    nextBaseHarnessId: string,
  ) => {
    setHasChanges(
      nextName !== (organization?.name || "") ||
        nextDefaultHarnessId !== (organization?.default_harness_id || "") ||
        nextBaseHarnessId !== (organization?.base_harness_id || ""),
    );
  };

  useEffect(() => {
    if (!organization) {
      return;
    }

    const genericHarnessId = harnesses.find((h) => h.name === "Generic")?.id || "";
    const baseHarnessFallbackId = harnesses.find((h) => h.name === "Base")?.id || "";

    setName(organization.name);
    setDefaultHarnessId(organization.default_harness_id || genericHarnessId);
    setBaseHarnessId(organization.base_harness_id || baseHarnessFallbackId);
    setHasChanges(false);
  }, [organization, harnesses]);

  const handleNameChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const nextName = e.target.value;
    setName(nextName);
    syncHasChanges(nextName, defaultHarnessId, baseHarnessId);
  };

  const handleSave = async () => {
    if (!hasChanges || !name.trim() || !defaultHarnessId || !baseHarnessId) return;

    await updateOrganization.mutateAsync({
      name: name.trim(),
      default_harness_id: defaultHarnessId,
      base_harness_id: baseHarnessId,
    });
    setHasChanges(false);
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter" && hasChanges && name.trim() && defaultHarnessId && baseHarnessId) {
      handleSave();
    }
  };

  if (error) {
    return (
      <div className="space-y-8">
        <section>
          <div className="mb-4">
            <h2 className="text-xl font-semibold">Organisation</h2>
            <p className="text-sm text-muted-foreground">Manage organisation settings.</p>
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
      {/* Current Organisation Settings */}
      <section>
        <div className="mb-6">
          <h2 className="text-xl font-semibold">Organisation</h2>
          <p className="text-sm text-muted-foreground">Manage organisation settings.</p>
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
              <div className="p-2 bg-primary/10">
                <Building2 className="h-5 w-5 text-primary" />
              </div>
              <div>
                <h3 className="font-medium">{organization?.name || currentOrg?.name}</h3>
                <p className="text-sm text-muted-foreground">Organisation settings</p>
              </div>
            </div>

            <div className="space-y-6 max-w-md">
              <div className="space-y-2">
                <Label htmlFor="org-name">Organisation Name</Label>
                <Input
                  id="org-name"
                  value={name}
                  onChange={handleNameChange}
                  onKeyDown={handleKeyDown}
                  placeholder="Organisation name"
                  className="flex-1"
                />
              </div>

              <div className="space-y-2">
                <Label htmlFor="default-harness">Default Harness</Label>
                <HarnessSelect
                  value={defaultHarnessId}
                  onValueChange={(value) => {
                    setDefaultHarnessId(value);
                    syncHasChanges(name, value, baseHarnessId);
                  }}
                  placeholder="Select default harness"
                />
                <p className="text-xs text-muted-foreground">
                  Used as the normal starting harness in the UI. New orgs default this to Generic.
                </p>
              </div>

              <div className="space-y-2">
                <Label htmlFor="base-harness">Base Harness</Label>
                <HarnessSelect
                  value={baseHarnessId}
                  onValueChange={(value) => {
                    setBaseHarnessId(value);
                    syncHasChanges(name, defaultHarnessId, value);
                  }}
                  placeholder="Select base harness"
                />
                <p className="text-xs text-muted-foreground">
                  Used only when a session starts without an explicit harness. New orgs default this
                  to Base.
                </p>
              </div>

              <div className="flex items-center gap-2">
                <Button
                  onClick={handleSave}
                  disabled={
                    !hasChanges ||
                    !name.trim() ||
                    !defaultHarnessId ||
                    !baseHarnessId ||
                    updateOrganization.isPending
                  }
                >
                  <Save className="h-4 w-4 mr-2" />
                  {updateOrganization.isPending ? "Saving..." : "Save"}
                </Button>
                {harnesses.length === 0 && (
                  <p className="text-xs text-muted-foreground">
                    Harnesses will appear after org init.
                  </p>
                )}
              </div>

              {updateOrganization.isError && (
                <p className="text-sm text-destructive">
                  Failed to update: {updateOrganization.error.message}
                </p>
              )}
              {updateOrganization.isSuccess && !hasChanges && (
                <p className="text-sm text-green-600">Saved successfully</p>
              )}

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
