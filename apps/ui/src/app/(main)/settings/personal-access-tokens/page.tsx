"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
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
import { Notice, NoticeDescription, NoticeTitle } from "@/components/ui/notice";
import {
  usePersonalAccessTokens,
  useCreatePersonalAccessToken,
  useDeletePersonalAccessToken,
} from "@/hooks/use-auth";
import { usePageTitle } from "@/hooks";
import { useAuth } from "@/providers/auth-provider";
import { Plus, Key, Trash2, Copy, Check, Clock, ShieldAlert } from "lucide-react";
import type {
  PersonalAccessTokenListItem,
  CreatePersonalAccessTokenRequest,
} from "@/lib/api/types";
import { EntityIdentity } from "@/components/ui/entity-identity";

const TOKEN_EXPIRY_OPTIONS = [
  {
    value: "1",
    label: "1 day",
    expiresInDays: 1,
    description: "Short-lived token for temporary use.",
  },
  {
    value: "30",
    label: "30 days",
    expiresInDays: 30,
    description: "Reasonable default for ongoing integration work.",
  },
  {
    value: "90",
    label: "90 days",
    expiresInDays: 90,
    description: "Recommended balance between longevity and rotation.",
  },
  {
    value: "unrestricted",
    label: "Unrestricted (not recommended)",
    expiresInDays: undefined,
    description: "This token never expires. Use only when you own the rotation process.",
  },
] as const;

type TokenExpiryOption = (typeof TOKEN_EXPIRY_OPTIONS)[number];
type TokenExpiryValue = TokenExpiryOption["value"];

const DEFAULT_TOKEN_EXPIRY: TokenExpiryValue = "90";
const DEFAULT_TOKEN_EXPIRY_OPTION =
  TOKEN_EXPIRY_OPTIONS.find((option) => option.value === DEFAULT_TOKEN_EXPIRY) ??
  TOKEN_EXPIRY_OPTIONS[0];

function PersonalAccessTokenRow({
  token,
  onDelete,
}: {
  token: PersonalAccessTokenListItem;
  onDelete: (id: string) => void;
}) {
  const formatDate = (dateStr: string | undefined) => {
    if (!dateStr) return "Never";
    return new Date(dateStr).toLocaleDateString();
  };

  return (
    <div className="flex items-center justify-between p-3 border">
      <div className="flex items-center gap-3">
        <Key className="h-5 w-5 text-muted-foreground" />
        <div>
          <div className="font-medium">
            <EntityIdentity value={token.id}>{token.name}</EntityIdentity>
          </div>
          <div className="text-sm text-muted-foreground font-mono">{token.token_prefix}</div>
        </div>
      </div>
      <div className="flex items-center gap-4">
        <div className="text-sm text-muted-foreground">
          <Clock className="h-3 w-3 inline mr-1" />
          Last used: {formatDate(token.last_used_at)}
        </div>
        {token.expires_at && (
          <Badge variant="outline" className="text-xs">
            Expires: {formatDate(token.expires_at)}
          </Badge>
        )}
        <Button
          variant="ghost"
          size="sm"
          className="text-destructive"
          onClick={() => onDelete(token.id)}
        >
          <Trash2 className="h-4 w-4" />
        </Button>
      </div>
    </div>
  );
}

function CreatePersonalAccessTokenDialog({
  open,
  onOpenChange,
  onTokenCreated,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onTokenCreated: (token: string) => void;
}) {
  const [name, setName] = useState("");
  const [expiryPreset, setExpiryPreset] = useState<TokenExpiryValue>(DEFAULT_TOKEN_EXPIRY);

  const createToken = useCreatePersonalAccessToken();
  const selectedExpiryOption =
    TOKEN_EXPIRY_OPTIONS.find((option) => option.value === expiryPreset) ??
    DEFAULT_TOKEN_EXPIRY_OPTION;

  const resetForm = () => {
    setName("");
    setExpiryPreset(DEFAULT_TOKEN_EXPIRY);
  };

  const handleOpenChange = (nextOpen: boolean) => {
    onOpenChange(nextOpen);
    if (!nextOpen) {
      resetForm();
    }
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const data: CreatePersonalAccessTokenRequest = {
      name,
      expires_in_days: selectedExpiryOption.expiresInDays,
    };
    const result = await createToken.mutateAsync(data);
    onTokenCreated(result.token);
    resetForm();
  };

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Create personal access token</DialogTitle>
          <DialogDescription>
            Personal access tokens are tied to your user account, not to an organization. The token
            inherits access to every organization and resource available to your account.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="token-name">Name</Label>
            <Input
              id="token-name"
              value={name}
              onChange={(e: React.ChangeEvent<HTMLInputElement>) => setName(e.target.value)}
              placeholder="My personal access token"
              required
            />
          </div>
          <div className="space-y-2">
            <Label>Expiration</Label>
            <div className="grid grid-cols-2 gap-2" role="group" aria-label="Expiration">
              {TOKEN_EXPIRY_OPTIONS.map((option) => {
                const selected = option.value === expiryPreset;
                return (
                  <Button
                    key={option.value}
                    type="button"
                    variant={selected ? "default" : "outline"}
                    aria-pressed={selected}
                    className="h-auto justify-start whitespace-normal px-3 py-2 text-left"
                    onClick={() => setExpiryPreset(option.value)}
                  >
                    {option.label}
                  </Button>
                );
              })}
            </div>
            <p className="text-xs text-muted-foreground">{selectedExpiryOption.description}</p>
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => handleOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={createToken.isPending || !name}>
              {createToken.isPending ? "Creating..." : "Create token"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function ShowPersonalAccessTokenDialog({
  token,
  open,
  onOpenChange,
}: {
  token: string | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    if (token) {
      await navigator.clipboard.writeText(token);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>Personal access token created</DialogTitle>
          <DialogDescription>
            Copy your token now. You won&apos;t be able to see it again!
          </DialogDescription>
        </DialogHeader>
        <div className="flex items-center gap-2">
          <div className="bg-muted p-3 font-mono text-sm flex-1 min-w-0 max-w-full whitespace-nowrap overflow-x-auto">
            {token}
          </div>
          <Button
            onClick={handleCopy}
            variant="outline"
            size="icon"
            className="shrink-0"
            aria-label={copied ? "Token copied" : "Copy token"}
            title={copied ? "Token copied" : "Copy token"}
          >
            {copied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
          </Button>
        </div>
        <DialogFooter>
          <Button onClick={() => onOpenChange(false)}>Done</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export default function PersonalAccessTokensPage() {
  usePageTitle("Personal access tokens", "Settings");
  const { requiresAuth } = useAuth();
  const {
    data: userTokens,
    isLoading: tokensLoading,
    error: tokensError,
  } = usePersonalAccessTokens();
  const deleteToken = useDeletePersonalAccessToken();

  const [createTokenOpen, setCreateTokenOpen] = useState(false);
  const [newToken, setNewToken] = useState<string | null>(null);

  const handleDeleteToken = async (id: string) => {
    if (
      confirm(
        "Are you sure you want to delete this personal access token? This action cannot be undone.",
      )
    ) {
      await deleteToken.mutateAsync(id);
    }
  };

  const handleTokenCreated = (token: string) => {
    setCreateTokenOpen(false);
    setNewToken(token);
  };

  // If auth is not required, show a message
  if (!requiresAuth) {
    return (
      <div className="space-y-8">
        <section>
          <div className="mb-4">
            <h2 className="text-xl font-semibold">Personal access tokens</h2>
            <p className="text-sm text-muted-foreground">
              Manage your personal access tokens for programmatic access.
            </p>
          </div>
          <Card className="p-8 text-center">
            <ShieldAlert className="h-12 w-12 mx-auto text-muted-foreground mb-4" />
            <h3 className="text-lg font-medium mb-2">Authentication Disabled</h3>
            <p className="text-muted-foreground">
              Personal access tokens are only available when authentication is enabled. Contact your
              administrator to enable authentication.
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
            <h2 className="text-xl font-semibold">Personal access tokens</h2>
            <p className="text-sm text-muted-foreground">
              Manage your personal access tokens for programmatic access. Tokens are tied to your
              user account, not to an organization.
            </p>
          </div>
          <Button onClick={() => setCreateTokenOpen(true)}>
            <Plus className="h-4 w-4 mr-2" />
            Create token
          </Button>
        </div>

        <Notice
          variant="warning"
          icon={<ShieldAlert className="h-5 w-5 shrink-0" aria-hidden="true" />}
          className="mb-4"
        >
          <NoticeTitle>Full account access</NoticeTitle>
          <NoticeDescription>
            Personal access tokens are tied to your user account (not an organization) and grant
            access to every organization and resource available to your account. Treat them like
            passwords &mdash; do not share or commit them to source control.
          </NoticeDescription>
        </Notice>

        <QueryStateWrapper
          isLoading={tokensLoading}
          error={tokensError}
          data={userTokens}
          errorMessagePrefix="Failed to load personal access tokens"
          loadingSkeleton={
            <div className="space-y-2">
              {[...Array(2)].map((_, i) => (
                <Skeleton key={i} className="h-16 w-full" />
              ))}
            </div>
          }
          emptyState={
            <Card className="p-8 text-center">
              <Key className="h-12 w-12 mx-auto text-muted-foreground mb-4" />
              <h3 className="text-lg font-medium mb-2">No personal access tokens</h3>
              <p className="text-muted-foreground mb-4">
                Create a personal access token to access the Everruns API programmatically.
              </p>
              <Button onClick={() => setCreateTokenOpen(true)}>
                <Plus className="h-4 w-4 mr-2" />
                Create token
              </Button>
            </Card>
          }
        >
          {(items) => (
            <div className="space-y-2">
              {items.map((token) => (
                <PersonalAccessTokenRow key={token.id} token={token} onDelete={handleDeleteToken} />
              ))}
            </div>
          )}
        </QueryStateWrapper>
      </section>

      {/* Dialogs */}
      <CreatePersonalAccessTokenDialog
        open={createTokenOpen}
        onOpenChange={setCreateTokenOpen}
        onTokenCreated={handleTokenCreated}
      />
      <ShowPersonalAccessTokenDialog
        token={newToken}
        open={newToken !== null}
        onOpenChange={(open) => !open && setNewToken(null)}
      />
    </div>
  );
}
