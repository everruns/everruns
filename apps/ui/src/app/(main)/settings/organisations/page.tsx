"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { Building2, Check, Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useCreateOrganization } from "@/hooks/use-organizations";
import { useOrg } from "@/providers/org-provider";

export default function OrganisationsPage() {
  const router = useRouter();
  const { currentOrg, organizations, setCurrentOrg } = useOrg();
  const createOrganization = useCreateOrganization();
  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const [newOrgName, setNewOrgName] = useState("");

  const handleCreateOrg = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!newOrgName.trim()) return;

    const org = await createOrganization.mutateAsync({
      name: newOrgName.trim(),
    });
    setCurrentOrg({ public_id: org.id, name: org.name, role: "owner" });
    setNewOrgName("");
    setCreateDialogOpen(false);
    router.push(`/orgs/${org.id}/setup`);
  };

  return (
    <div className="space-y-8">
      <section>
        <div className="mb-6 flex items-center justify-between gap-4">
          <div>
            <h2 className="text-xl font-semibold">Organisations</h2>
            <p className="text-sm text-muted-foreground">All organisations you are a member of.</p>
          </div>
          <Button onClick={() => setCreateDialogOpen(true)}>
            <Plus className="h-4 w-4 mr-2" />
            Create Organisation
          </Button>
        </div>

        {organizations.length === 0 ? (
          <Card className="p-8 text-center">
            <Building2 className="h-12 w-12 mx-auto text-muted-foreground mb-4" />
            <h3 className="text-lg font-medium mb-2">No organisations</h3>
            <p className="text-muted-foreground">
              Create an organisation to start managing shared resources.
            </p>
          </Card>
        ) : (
          <div className="space-y-3">
            {organizations.map((org) => {
              const isCurrent = currentOrg?.public_id === org.public_id;
              return (
                <Card
                  key={org.public_id}
                  className={`p-4 flex items-center justify-between gap-4 ${isCurrent ? "border-primary/50" : ""}`}
                >
                  <div className="flex min-w-0 items-center gap-3">
                    <div className={`p-2 ${isCurrent ? "bg-primary/10" : "bg-muted"}`}>
                      <Building2
                        className={`h-4 w-4 ${isCurrent ? "text-primary" : "text-muted-foreground"}`}
                      />
                    </div>
                    <div className="min-w-0">
                      <p className="truncate font-medium">{org.name}</p>
                      <p className="truncate text-xs text-muted-foreground font-mono">
                        {org.public_id}
                      </p>
                    </div>
                  </div>
                  <div className="flex shrink-0 items-center gap-2">
                    {isCurrent ? (
                      <span className="flex items-center gap-1 text-sm text-primary">
                        <Check className="h-4 w-4" />
                        Current
                      </span>
                    ) : (
                      <Button variant="outline" size="sm" onClick={() => setCurrentOrg(org)}>
                        Switch
                      </Button>
                    )}
                  </div>
                </Card>
              );
            })}
          </div>
        )}
      </section>

      <Dialog open={createDialogOpen} onOpenChange={setCreateDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Create Organisation</DialogTitle>
            <DialogDescription>
              Create a new organisation. You will be added as a member automatically.
            </DialogDescription>
          </DialogHeader>
          <form onSubmit={handleCreateOrg} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="new-org-name">Name</Label>
              <Input
                id="new-org-name"
                value={newOrgName}
                onChange={(e) => setNewOrgName(e.target.value)}
                placeholder="Organisation name"
                required
              />
            </div>
            {createOrganization.isError && (
              <p className="text-sm text-destructive">
                Failed to create: {createOrganization.error.message}
              </p>
            )}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setCreateDialogOpen(false)}>
                Cancel
              </Button>
              <Button type="submit" disabled={createOrganization.isPending || !newOrgName.trim()}>
                {createOrganization.isPending ? "Creating..." : "Create"}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </div>
  );
}
