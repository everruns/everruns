"use client";

import { useState, useMemo } from "react";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar";
import { useAuth } from "@/providers/auth-provider";
import { useUsers } from "@/hooks/use-users";
import { Users, Search, ShieldAlert, Mail, Calendar, Shield } from "lucide-react";
import type { User } from "@/lib/api/types";

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

function getAuthProviderLabel(provider?: string): string {
  if (!provider) return "Local";
  switch (provider) {
    case "google":
      return "Google";
    case "github":
      return "GitHub";
    case "local":
      return "Local";
    default:
      return provider;
  }
}

function UserCard({ user }: { user: User }) {
  const isAdmin = user.roles.includes("admin");

  return (
    <div className="flex items-center justify-between p-4 border rounded-lg">
      <div className="flex items-center gap-4">
        <Avatar className="h-10 w-10">
          {user.avatar_url && <AvatarImage src={user.avatar_url} alt={user.name} />}
          <AvatarFallback>{getInitials(user.name)}</AvatarFallback>
        </Avatar>
        <div>
          <div className="font-medium flex items-center gap-2">
            {user.name}
            {isAdmin && (
              <Badge variant="secondary" className="text-xs">
                <Shield className="h-3 w-3 mr-1" />
                Admin
              </Badge>
            )}
          </div>
          <div className="flex items-center gap-4 text-sm text-muted-foreground">
            <span className="flex items-center gap-1">
              <Mail className="h-3 w-3" />
              {user.email}
            </span>
          </div>
        </div>
      </div>
      <div className="flex items-center gap-4 text-sm text-muted-foreground">
        <Badge variant="outline">{getAuthProviderLabel(user.auth_provider)}</Badge>
        <span className="flex items-center gap-1">
          <Calendar className="h-3 w-3" />
          Joined {formatDate(user.created_at)}
        </span>
      </div>
    </div>
  );
}

function UserCardSkeleton() {
  return (
    <div className="flex items-center justify-between p-4 border rounded-lg">
      <div className="flex items-center gap-4">
        <Skeleton className="h-10 w-10 rounded-full" />
        <div className="space-y-2">
          <Skeleton className="h-4 w-32" />
          <Skeleton className="h-3 w-48" />
        </div>
      </div>
      <div className="flex items-center gap-4">
        <Skeleton className="h-5 w-16" />
        <Skeleton className="h-3 w-24" />
      </div>
    </div>
  );
}

export default function MembersPage() {
  const { requiresAuth } = useAuth();
  const [searchQuery, setSearchQuery] = useState("");

  // Debounce the search query for API calls
  const [debouncedSearch, setDebouncedSearch] = useState("");

  // Simple debounce effect
  useMemo(() => {
    const timer = setTimeout(() => {
      setDebouncedSearch(searchQuery);
    }, 300);
    return () => clearTimeout(timer);
  }, [searchQuery]);

  const {
    data: users = [],
    isLoading,
    error,
  } = useUsers(debouncedSearch ? { search: debouncedSearch } : undefined);

  // If auth is not required, show a message
  if (!requiresAuth) {
    return (
      <div className="space-y-8">
        <section>
          <div className="mb-4">
            <h2 className="text-xl font-semibold">Members</h2>
            <p className="text-sm text-muted-foreground">
              View and manage team members.
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
              View and manage team members.
            </p>
          </div>
        </div>

        {/* Search */}
        <div className="relative mb-4">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
          <Input
            placeholder="Search by name or email..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className="pl-10"
          />
        </div>

        {error && (
          <div className="bg-destructive/10 text-destructive p-4 rounded-lg mb-4">
            Failed to load members: {error.message}
          </div>
        )}

        {isLoading ? (
          <div className="space-y-2">
            {[...Array(3)].map((_, i) => (
              <UserCardSkeleton key={i} />
            ))}
          </div>
        ) : users.length === 0 ? (
          <Card className="p-8 text-center">
            <Users className="h-12 w-12 mx-auto text-muted-foreground mb-4" />
            <h3 className="text-lg font-medium mb-2">
              {searchQuery ? "No members found" : "No members"}
            </h3>
            <p className="text-muted-foreground">
              {searchQuery
                ? `No members match "${searchQuery}". Try a different search.`
                : "No team members have been added yet."}
            </p>
          </Card>
        ) : (
          <div className="space-y-2">
            <div className="text-sm text-muted-foreground mb-2">
              {users.length} member{users.length !== 1 ? "s" : ""}
              {searchQuery && ` matching "${searchQuery}"`}
            </div>
            {users.map((user) => (
              <UserCard key={user.id} user={user} />
            ))}
          </div>
        )}
      </section>
    </div>
  );
}
