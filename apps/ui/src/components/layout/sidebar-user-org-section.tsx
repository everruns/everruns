/**
 * Organization section rendered inside the bottom user menu (Direction F).
 *
 * Orgs are switched rarely, so they live here rather than in the prime top
 * slot: current org, switch between orgs, org settings, and create a new org.
 */
"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { Building2, Check, Plus, Settings } from "lucide-react";
import { useCreateOrganization } from "@/hooks/use-organizations";
import { useOrg } from "@/providers/org-provider";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
} from "@/components/ui/dropdown-menu";

function CreateOrganizationDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [name, setName] = useState("");
  const createOrg = useCreateOrganization();
  const { setCurrentOrg } = useOrg();
  const router = useRouter();

  const handleSubmit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!name.trim()) return;
    const org = await createOrg.mutateAsync({ name: name.trim() });
    setCurrentOrg({ public_id: org.id, name: org.name, role: "owner" });
    setName("");
    onOpenChange(false);
    router.push(`/orgs/${org.id}/setup`);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Create organization</DialogTitle>
          <DialogDescription>
            An organization is the billing and team boundary. You will be added as a member
            automatically.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="org-name">Name</Label>
            <Input
              id="org-name"
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder="Organization name"
              required
            />
          </div>
          {createOrg.isError && (
            <p className="text-sm text-destructive">
              Failed to create organization: {createOrg.error.message}
            </p>
          )}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={createOrg.isPending || !name.trim()}>
              {createOrg.isPending ? "Creating..." : "Create"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export function SidebarUserOrgSection() {
  const { currentOrg, organizations, setCurrentOrg } = useOrg();
  const [createOpen, setCreateOpen] = useState(false);
  const router = useRouter();

  return (
    <>
      <DropdownMenuGroup>
        <DropdownMenuLabel>Organization</DropdownMenuLabel>
        {organizations.map((org) => (
          <DropdownMenuItem
            key={org.public_id}
            onClick={() => setCurrentOrg(org)}
            className="flex items-center justify-between"
          >
            <span className="flex min-w-0 items-center gap-2">
              <Building2 className="icon-sharp h-4 w-4 shrink-0 text-muted-foreground" />
              <span className="truncate">{org.name}</span>
            </span>
            {currentOrg?.public_id === org.public_id && (
              <Check className="icon-sharp h-4 w-4 text-primary" />
            )}
          </DropdownMenuItem>
        ))}
        <DropdownMenuItem onClick={() => router.push("/settings/organization")}>
          <Settings className="icon-sharp mr-2 h-4 w-4" />
          Organization settings
        </DropdownMenuItem>
        <DropdownMenuItem onClick={() => setCreateOpen(true)}>
          <Plus className="icon-sharp mr-2 h-4 w-4" />
          New organization
        </DropdownMenuItem>
      </DropdownMenuGroup>

      <CreateOrganizationDialog open={createOpen} onOpenChange={setCreateOpen} />
    </>
  );
}
