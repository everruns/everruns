/**
 * Decisions:
 * - Org switching stays in a dedicated menu so SaaS forks can swap creation behavior without touching sidebar layout.
 * - Creation dialog owns post-create redirect because that flow is specific to organization bootstrap.
 */
"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { Building2, Check, ChevronDown, Plus } from "lucide-react";
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
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuPositioner,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
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
          <DialogTitle>Create Organisation</DialogTitle>
          <DialogDescription>
            Create a new organisation. You will be added as a member
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
              placeholder="Organisation name"
              required
            />
          </div>
          {createOrg.isError && (
            <p className="text-sm text-destructive">
              Failed to create organisation: {createOrg.error.message}
            </p>
          )}
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
            >
              Cancel
            </Button>
            <Button
              type="submit"
              disabled={createOrg.isPending || !name.trim()}
            >
              {createOrg.isPending ? "Creating..." : "Create"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export function SidebarOrganizationMenu({
  onCreateOrg,
  useDefaultCreateOrgDialog,
}: {
  onCreateOrg: () => void;
  useDefaultCreateOrgDialog: boolean;
}) {
  const { currentOrg, organizations, setCurrentOrg } = useOrg();
  const [createOrgOpen, setCreateOrgOpen] = useState(false);

  if (organizations.length === 0) return null;

  const handleCreate = () => {
    if (useDefaultCreateOrgDialog) {
      setCreateOrgOpen(true);
      return;
    }
    onCreateOrg();
  };

  return (
    <>
      <div className="border-b px-3 py-2">
        <DropdownMenu>
          <DropdownMenuTrigger className="flex w-full items-center gap-2 border border-transparent px-3 py-2 text-sm transition-colors hover:border-border hover:bg-card">
            <Building2 className="icon-sharp h-4 w-4 text-muted-foreground" />
            <span className="flex-1 truncate text-left font-medium">
              {currentOrg?.name ?? "Select Organization"}
            </span>
            <ChevronDown className="icon-sharp h-4 w-4 text-muted-foreground" />
          </DropdownMenuTrigger>
          <DropdownMenuPositioner side="bottom" align="start">
            <DropdownMenuContent className="w-56">
              <DropdownMenuGroup>
                <DropdownMenuLabel>Organizations</DropdownMenuLabel>
                {organizations.map((org) => (
                  <DropdownMenuItem
                    key={org.public_id}
                    onClick={() => setCurrentOrg(org)}
                    className="flex items-center justify-between"
                  >
                    <span className="truncate">{org.name}</span>
                    {currentOrg?.public_id === org.public_id && (
                      <Check className="icon-sharp h-4 w-4 text-primary" />
                    )}
                  </DropdownMenuItem>
                ))}
              </DropdownMenuGroup>
              <DropdownMenuSeparator />
              <DropdownMenuItem onClick={handleCreate}>
                <Plus className="icon-sharp mr-2 h-4 w-4" />
                Create Organisation
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenuPositioner>
        </DropdownMenu>
      </div>

      {useDefaultCreateOrgDialog && (
        <CreateOrganizationDialog
          open={createOrgOpen}
          onOpenChange={setCreateOrgOpen}
        />
      )}
    </>
  );
}
