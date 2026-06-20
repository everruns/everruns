"use client";

import { useEffect, useState } from "react";
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
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useCreateInvite } from "@/hooks/use-invitations";
import type { CreatedInvitation, EmailDelivery } from "@/lib/api/invitations";
import type { OrgRole } from "@/lib/api/types";
import { AlertCircle, Check, Copy, MailCheck } from "lucide-react";

interface InviteMemberDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Owners may invite owners; admins may not. */
  canInviteOwner: boolean;
}

const DELIVERY_MESSAGE: Record<EmailDelivery, string> = {
  sent: "We emailed the invitation. You can also share the link below.",
  failed: "We couldn't send the email. Share this invite link instead — it works the same way.",
  not_configured:
    "Email delivery isn't configured for this deployment. Share this invite link to add the member.",
};

export function InviteMemberDialog({
  open,
  onOpenChange,
  canInviteOwner,
}: InviteMemberDialogProps) {
  const [email, setEmail] = useState("");
  const [role, setRole] = useState<OrgRole>("member");
  const [error, setError] = useState<string | null>(null);
  const [created, setCreated] = useState<CreatedInvitation | null>(null);
  const [copied, setCopied] = useState(false);
  const createInvite = useCreateInvite();

  useEffect(() => {
    if (open) {
      setEmail("");
      setRole("member");
      setError(null);
      setCreated(null);
      setCopied(false);
    }
  }, [open]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    try {
      const result = await createInvite.mutateAsync({ email: email.trim(), role });
      setCreated(result);
    } catch (err) {
      const apiError = (err as { response?: { data?: string } })?.response?.data;
      const message = err instanceof Error ? err.message : "Failed to create invitation";
      setError(typeof apiError === "string" ? apiError : message);
    }
  };

  const handleCopy = async () => {
    if (!created) return;
    try {
      await navigator.clipboard.writeText(created.invite_url);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      setError("Could not copy to clipboard. Copy the link manually.");
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[480px]">
        {created ? (
          <>
            <DialogHeader>
              <DialogTitle className="flex items-center gap-2">
                <MailCheck className="h-5 w-5" />
                Invitation created
              </DialogTitle>
              <DialogDescription>
                {created.email} was invited as {created.role}.
              </DialogDescription>
            </DialogHeader>

            <p className="text-sm text-muted-foreground">
              {DELIVERY_MESSAGE[created.email_delivery]}
            </p>

            <div className="space-y-2">
              <Label htmlFor="invite-link">Invite link</Label>
              <div className="flex items-center gap-2">
                <Input
                  id="invite-link"
                  readOnly
                  value={created.invite_url}
                  onFocus={(e) => e.currentTarget.select()}
                  className="font-mono text-xs"
                />
                <Button type="button" variant="outline" size="icon" onClick={handleCopy}>
                  {copied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
                </Button>
              </div>
              <p className="text-xs text-muted-foreground">
                This link is shown only once and expires when the invitation does.
              </p>
            </div>

            {error && (
              <div className="flex items-center gap-2 text-destructive text-sm">
                <AlertCircle className="h-4 w-4 flex-shrink-0" />
                {error}
              </div>
            )}

            <DialogFooter>
              <Button type="button" onClick={() => onOpenChange(false)}>
                Done
              </Button>
            </DialogFooter>
          </>
        ) : (
          <form onSubmit={handleSubmit}>
            <DialogHeader>
              <DialogTitle>Invite a member</DialogTitle>
              <DialogDescription>
                Send an invitation by email, or share the generated invite link.
              </DialogDescription>
            </DialogHeader>

            <div className="space-y-4 my-4">
              <div className="space-y-2">
                <Label htmlFor="invite-email">Email</Label>
                <Input
                  id="invite-email"
                  type="email"
                  required
                  autoComplete="off"
                  placeholder="teammate@example.com"
                  value={email}
                  onChange={(e) => setEmail(e.target.value)}
                />
              </div>

              <div className="space-y-2">
                <Label htmlFor="invite-role">Role</Label>
                <Select value={role} onValueChange={(v) => setRole(v as OrgRole)}>
                  <SelectTrigger id="invite-role" className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="member">Member</SelectItem>
                    <SelectItem value="admin">Admin</SelectItem>
                    {canInviteOwner && <SelectItem value="owner">Owner</SelectItem>}
                  </SelectContent>
                </Select>
              </div>
            </div>

            {error && (
              <div className="flex items-center gap-2 text-destructive text-sm mb-4">
                <AlertCircle className="h-4 w-4 flex-shrink-0" />
                {error}
              </div>
            )}

            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
                Cancel
              </Button>
              <Button type="submit" disabled={createInvite.isPending}>
                {createInvite.isPending ? "Creating..." : "Create invitation"}
              </Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
}
