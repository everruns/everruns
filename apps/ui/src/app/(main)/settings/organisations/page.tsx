"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { Building2, Check, Plus } from "lucide-react";
import { Badge } from "@/components/ui/badge";
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
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useCreateOrganization } from "@/hooks/use-organizations";
import { useOrg } from "@/providers/org-provider";
import type { OrgRole } from "@/lib/api/types";

const ROLE_LABELS: Record<OrgRole, string> = {
  owner: "Owner",
  admin: "Admin",
  member: "Member",
};

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
          <Card className="overflow-hidden">
            <Table aria-label="Organisations">
              <TableHeader>
                <TableRow>
                  <TableHead>Organisation</TableHead>
                  <TableHead>ID</TableHead>
                  <TableHead>Role</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead className="text-right">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {organizations.map((org) => {
                  const isCurrent = currentOrg?.public_id === org.public_id;
                  return (
                    <TableRow key={org.public_id} data-state={isCurrent ? "selected" : undefined}>
                      <TableCell className="font-medium">{org.name}</TableCell>
                      <TableCell className="font-mono text-xs text-muted-foreground">
                        {org.public_id}
                      </TableCell>
                      <TableCell>{ROLE_LABELS[org.role]}</TableCell>
                      <TableCell>
                        {isCurrent ? (
                          <Badge variant="accent">
                            <Check className="h-3 w-3" />
                            Current
                          </Badge>
                        ) : (
                          <span className="text-muted-foreground">Available</span>
                        )}
                      </TableCell>
                      <TableCell className="text-right">
                        <Button
                          variant="outline"
                          size="sm"
                          disabled={isCurrent}
                          aria-label={`Switch to ${org.name}`}
                          onClick={() => setCurrentOrg(org)}
                        >
                          Switch
                        </Button>
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          </Card>
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
