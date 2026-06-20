"use client";

import { useState } from "react";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useAuth } from "@/providers/auth-provider";
import { useOrg } from "@/providers/org-provider";
import { useMembers, useUpdateMemberRole, useRemoveMember } from "@/hooks/use-members";
import { useInvitations, useRevokeInvite } from "@/hooks/use-invitations";
import { usePageTitle } from "@/hooks";
import {
  Users,
  Shield,
  ShieldCheck,
  Crown,
  UserMinus,
  Loader2,
  UserPlus,
  Mail,
  Clock,
  X,
} from "lucide-react";
import type { OrgMember } from "@/lib/api/members";
import type { Invitation, InvitationStatus } from "@/lib/api/invitations";
import type { OrgRole } from "@/lib/api/types";
import { InviteMemberDialog } from "@/components/invitations/invite-member-dialog";

function getInitials(name: string): string {
  return name
    .split(" ")
    .map((n) => n[0])
    .join("")
    .toUpperCase()
    .slice(0, 2);
}

function formatDate(dateStr: string): string {
  return new Date(dateStr).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

const ROLE_LABELS: Record<OrgRole, string> = {
  owner: "Owner",
  admin: "Admin",
  member: "Member",
};

const ROLE_ICONS: Record<OrgRole, typeof Crown> = {
  owner: Crown,
  admin: ShieldCheck,
  member: Shield,
};

const ROLE_BADGE_VARIANT: Record<OrgRole, "default" | "secondary" | "outline"> = {
  owner: "default",
  admin: "secondary",
  member: "outline",
};

function MemberCard({
  member,
  currentUserId,
  canManage,
  isOwner,
}: {
  member: OrgMember;
  currentUserId: string;
  canManage: boolean;
  isOwner: boolean;
}) {
  const updateRole = useUpdateMemberRole();
  const removeMember = useRemoveMember();
  const [pendingRole, setPendingRole] = useState<string | null>(null);
  const isSelf = member.user_id === currentUserId;
  const RoleIcon = ROLE_ICONS[member.role];

  const handleRoleChange = async (newRole: string) => {
    setPendingRole(newRole);
    try {
      await updateRole.mutateAsync({ userId: member.user_id, role: newRole as OrgRole });
    } catch {
      // Error handled by mutation
    } finally {
      setPendingRole(null);
    }
  };

  const handleRemove = async () => {
    if (!confirm(`Remove ${member.name} from this organization?`)) return;
    try {
      await removeMember.mutateAsync(member.user_id);
    } catch {
      // Error handled by mutation
    }
  };

  return (
    <div className="flex items-center justify-between p-4 border">
      <div className="flex items-center gap-4">
        <Avatar className="h-10 w-10">
          {member.avatar_url && <AvatarImage src={member.avatar_url} alt={member.name} />}
          <AvatarFallback>{getInitials(member.name)}</AvatarFallback>
        </Avatar>
        <div>
          <div className="font-medium flex items-center gap-2">
            {member.name}
            {isSelf && (
              <Badge variant="outline" className="text-xs">
                You
              </Badge>
            )}
          </div>
          <div className="text-sm text-muted-foreground">{member.email}</div>
        </div>
      </div>
      <div className="flex items-center gap-3">
        <span className="text-xs text-muted-foreground hidden sm:inline">
          Joined {formatDate(member.joined_at)}
        </span>

        {canManage && !isSelf ? (
          <Select
            value={pendingRole ?? member.role}
            onValueChange={handleRoleChange}
            disabled={updateRole.isPending}
          >
            <SelectTrigger className="w-[120px]">
              {updateRole.isPending ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <SelectValue>{ROLE_LABELS[(pendingRole ?? member.role) as OrgRole]}</SelectValue>
              )}
            </SelectTrigger>
            <SelectContent>
              {isOwner && <SelectItem value="owner">Owner</SelectItem>}
              <SelectItem value="admin">Admin</SelectItem>
              <SelectItem value="member">Member</SelectItem>
            </SelectContent>
          </Select>
        ) : (
          <Badge variant={ROLE_BADGE_VARIANT[member.role]} className="gap-1">
            <RoleIcon className="h-3 w-3" />
            {ROLE_LABELS[member.role]}
          </Badge>
        )}

        {canManage && !isSelf && (
          <Button
            variant="ghost"
            size="icon"
            onClick={handleRemove}
            disabled={removeMember.isPending}
            title="Remove member"
          >
            {removeMember.isPending ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <UserMinus className="h-4 w-4 text-muted-foreground" />
            )}
          </Button>
        )}
      </div>
    </div>
  );
}

function MemberCardSkeleton() {
  return (
    <div className="flex items-center justify-between p-4 border">
      <div className="flex items-center gap-4">
        <Skeleton className="h-10 w-10 rounded-full" />
        <div className="space-y-2">
          <Skeleton className="h-4 w-32" />
          <Skeleton className="h-3 w-48" />
        </div>
      </div>
      <div className="flex items-center gap-3">
        <Skeleton className="h-5 w-16" />
        <Skeleton className="h-8 w-[120px]" />
      </div>
    </div>
  );
}

const INVITE_STATUS_LABEL: Record<InvitationStatus, string> = {
  pending: "Pending",
  expired: "Expired",
  accepted: "Accepted",
  revoked: "Revoked",
};

function InviteRow({ invite, canManage }: { invite: Invitation; canManage: boolean }) {
  const revokeInvite = useRevokeInvite();
  const isExpired = invite.status === "expired";

  const handleRevoke = async () => {
    if (!confirm(`Revoke the invitation for ${invite.email}?`)) return;
    try {
      await revokeInvite.mutateAsync(invite.id);
    } catch {
      // Error handled by mutation
    }
  };

  return (
    <div className="flex items-center justify-between p-4 border">
      <div className="flex items-center gap-4">
        <div className="flex h-10 w-10 items-center justify-center rounded-full bg-muted">
          <Mail className="h-5 w-5 text-muted-foreground" />
        </div>
        <div>
          <div className="font-medium">{invite.email}</div>
          <div className="text-sm text-muted-foreground capitalize">{invite.role}</div>
        </div>
      </div>
      <div className="flex items-center gap-3">
        <Badge variant={isExpired ? "outline" : "secondary"} className="gap-1">
          <Clock className="h-3 w-3" />
          {INVITE_STATUS_LABEL[invite.status]}
        </Badge>
        {canManage && (
          <Button
            variant="ghost"
            size="icon"
            onClick={handleRevoke}
            disabled={revokeInvite.isPending}
            title="Revoke invitation"
          >
            {revokeInvite.isPending ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <X className="h-4 w-4 text-muted-foreground" />
            )}
          </Button>
        )}
      </div>
    </div>
  );
}

function PendingInvitesSection({ canManage }: { canManage: boolean }) {
  const { data: invites, isLoading } = useInvitations();

  if (isLoading || !invites || invites.length === 0) return null;

  return (
    <section>
      <h2 className="text-xl font-semibold mb-1">Pending invitations</h2>
      <p className="text-sm text-muted-foreground mb-4">
        Invitations that haven&apos;t been accepted yet.
      </p>
      <div className="space-y-2">
        {invites.map((invite) => (
          <InviteRow key={invite.id} invite={invite} canManage={canManage} />
        ))}
      </div>
    </section>
  );
}

export default function MembersPage() {
  usePageTitle("Members", "Settings");
  const { user } = useAuth();
  const { currentOrg, hasRole } = useOrg();
  const { data: members, isLoading, error } = useMembers();
  const [inviteOpen, setInviteOpen] = useState(false);

  const canManage = hasRole("admin");
  const isOwner = hasRole("owner");
  const currentUserId = user?.id ?? "";

  return (
    <div className="space-y-8">
      <section>
        <div className="flex items-center justify-between mb-4">
          <div>
            <h2 className="text-xl font-semibold">Members</h2>
            <p className="text-sm text-muted-foreground">
              Manage members of{" "}
              <span className="font-medium">{currentOrg?.name ?? "your organization"}</span>.
            </p>
          </div>
          {canManage && (
            <Button onClick={() => setInviteOpen(true)} className="gap-2">
              <UserPlus className="h-4 w-4" />
              Invite member
            </Button>
          )}
        </div>

        <QueryStateWrapper
          isLoading={isLoading}
          error={error}
          data={members}
          errorMessagePrefix="Failed to load members"
          loadingSkeleton={
            <div className="space-y-2">
              {[...Array(3)].map((_, i) => (
                <MemberCardSkeleton key={i} />
              ))}
            </div>
          }
          emptyState={
            <Card className="p-8 text-center">
              <Users className="h-12 w-12 mx-auto text-muted-foreground mb-4" />
              <h3 className="text-lg font-medium mb-2">No members</h3>
              <p className="text-muted-foreground">
                No members have been added to this organization yet.
              </p>
            </Card>
          }
        >
          {(items) => (
            <div className="space-y-2">
              <div className="text-sm text-muted-foreground mb-2">
                {items.length} member{items.length !== 1 ? "s" : ""}
              </div>
              {items.map((member) => (
                <MemberCard
                  key={member.user_id}
                  member={member}
                  currentUserId={currentUserId}
                  canManage={canManage}
                  isOwner={isOwner}
                />
              ))}
            </div>
          )}
        </QueryStateWrapper>
      </section>

      <PendingInvitesSection canManage={canManage} />

      <InviteMemberDialog open={inviteOpen} onOpenChange={setInviteOpen} canInviteOwner={isOwner} />
    </div>
  );
}
