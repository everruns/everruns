"use client";

// Agent keys of an `api` channel: list, create, rotate, revoke.
//
// A secret is shown exactly once, in the dialog that created or rotated it; the
// list only ever has the display prefix. Keys belong to the organization, so
// any member who can manage the agent manages every key on the channel.
//
// This card renders inside the channel editor's <form>. Dialogs are portals,
// and React bubbles synthetic events through portals, so the name form below
// stops its submit event from reaching (and saving) the channel form.

import { useState } from "react";
import { KeyRound, Plus, RotateCw, ShieldOff } from "lucide-react";
import {
  useAgentKeys,
  useCreateAgentKey,
  useRevokeAgentKey,
  useRotateAgentKey,
} from "@/hooks/use-agent-channels";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { CopyButton } from "@/components/ui/copy-button";
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
import { Notice, NoticeDescription } from "@/components/ui/notice";
import type { AgentKey, AgentKeyWithSecret } from "@/lib/api/types";
import { formatRelativeTime } from "@/lib/formatting";

const DEFAULT_OVERLAP_HOURS = 24;
const MAX_OVERLAP_HOURS = 168;

type Pending = { action: "rotate" | "revoke"; key: AgentKey } | null;

function keyState(key: AgentKey, now: number): { label: string; live: boolean } {
  if (key.revoked_at) return { label: "Revoked", live: false };
  if (key.expires_at && Date.parse(key.expires_at) <= now) return { label: "Expired", live: false };
  return { label: "Active", live: true };
}

export function AgentKeysCard({
  agentId,
  channelId,
  canManage,
}: {
  agentId: string;
  channelId: string;
  canManage: boolean;
}) {
  const keys = useAgentKeys(agentId, channelId);
  const createKey = useCreateAgentKey(agentId, channelId);
  const rotateKey = useRotateAgentKey(agentId, channelId);
  const revokeKey = useRevokeAgentKey(agentId, channelId);
  const [createOpen, setCreateOpen] = useState(false);
  const [name, setName] = useState("");
  const [pending, setPending] = useState<Pending>(null);
  const [overlapHours, setOverlapHours] = useState(String(DEFAULT_OVERLAP_HOURS));
  const [revealed, setRevealed] = useState<AgentKeyWithSecret | null>(null);
  const now = Date.now();
  const overlap = Number.parseInt(overlapHours, 10);
  const overlapValid =
    /^\d+$/.test(overlapHours.trim()) && overlap >= 0 && overlap <= MAX_OVERLAP_HOURS;

  const closeCreate = () => {
    setCreateOpen(false);
    setName("");
    createKey.reset();
  };
  const closePending = () => {
    setPending(null);
    setOverlapHours(String(DEFAULT_OVERLAP_HOURS));
    rotateKey.reset();
    revokeKey.reset();
  };

  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between gap-3">
        <CardTitle className="flex items-center gap-2">
          <KeyRound className="size-4" />
          Agent keys
        </CardTitle>
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={!canManage}
          onClick={() => setCreateOpen(true)}
        >
          <Plus className="size-4" />
          Create key
        </Button>
      </CardHeader>
      <CardContent className="space-y-3">
        <p className="text-sm text-muted-foreground">
          Your code sends a key as <code>Authorization: Bearer evr_ak_…</code>. A key reaches only
          this agent&apos;s session routes, never the management API, and sees only the sessions it
          started.
        </p>
        {keys.isLoading && <p className="text-sm text-muted-foreground">Loading keys...</p>}
        {keys.error && (
          <Notice variant="destructive">
            <NoticeDescription>{keys.error.message}</NoticeDescription>
          </Notice>
        )}
        {keys.data && keys.data.length === 0 && (
          <p className="text-sm text-muted-foreground">
            No keys yet. Create one for each application that calls this agent.
          </p>
        )}
        {keys.data && keys.data.length > 0 && (
          <ul className="divide-y border" aria-label="Agent keys">
            {keys.data.map((key) => {
              const state = keyState(key, now);
              const overlapActive =
                state.live &&
                key.previous_valid_until &&
                Date.parse(key.previous_valid_until) > now;
              return (
                <li key={key.id} className="flex flex-wrap items-center gap-3 p-3">
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="truncate font-medium">{key.name}</span>
                      <Badge variant={state.live ? "default" : "secondary"}>{state.label}</Badge>
                    </div>
                    <p className="mt-1 font-mono text-xs text-muted-foreground">{key.prefix}</p>
                    <p className="mt-1 text-xs text-muted-foreground">
                      {key.last_used_at
                        ? `Last used ${formatRelativeTime(key.last_used_at)}`
                        : "Never used"}
                      {overlapActive && key.previous_valid_until
                        ? ` · previous secret works until ${new Date(key.previous_valid_until).toLocaleString()}`
                        : ""}
                    </p>
                  </div>
                  {state.live && (
                    <div className="flex gap-2">
                      <Button
                        type="button"
                        size="sm"
                        variant="outline"
                        disabled={!canManage}
                        onClick={() => setPending({ action: "rotate", key })}
                      >
                        <RotateCw className="size-4" />
                        Rotate
                      </Button>
                      <Button
                        type="button"
                        size="sm"
                        variant="outline"
                        disabled={!canManage}
                        onClick={() => setPending({ action: "revoke", key })}
                      >
                        <ShieldOff className="size-4" />
                        Revoke
                      </Button>
                    </div>
                  )}
                </li>
              );
            })}
          </ul>
        )}
      </CardContent>

      <Dialog
        open={createOpen}
        onOpenChange={(open) => (open ? setCreateOpen(true) : closeCreate())}
      >
        <DialogContent>
          <form
            onSubmit={(event) => {
              event.preventDefault();
              event.stopPropagation();
              if (!name.trim()) return;
              createKey.mutate(name.trim(), {
                onSuccess: (created) => {
                  closeCreate();
                  setRevealed(created);
                },
              });
            }}
            className="space-y-4"
          >
            <DialogHeader>
              <DialogTitle>Create agent key</DialogTitle>
              <DialogDescription>
                Name it after the application that will hold it, so you know which key to revoke.
              </DialogDescription>
            </DialogHeader>
            <div className="space-y-2">
              <Label htmlFor="agent_key_name">Name</Label>
              <Input
                id="agent_key_name"
                value={name}
                onChange={(event) => setName(event.target.value)}
                placeholder="Support backend"
                autoFocus
              />
            </div>
            {createKey.error && (
              <Notice variant="destructive">
                <NoticeDescription>{createKey.error.message}</NoticeDescription>
              </Notice>
            )}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={closeCreate}>
                Cancel
              </Button>
              <Button type="submit" disabled={!name.trim() || createKey.isPending}>
                {createKey.isPending ? "Creating..." : "Create key"}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      <Dialog open={pending !== null} onOpenChange={(open) => !open && closePending()}>
        <DialogContent>
          {pending?.action === "rotate" && (
            <>
              <DialogHeader>
                <DialogTitle>Rotate {pending.key.name}</DialogTitle>
                <DialogDescription>
                  You get a new secret for the same key. The current secret keeps working for the
                  overlap, so you can deploy the new one first.
                </DialogDescription>
              </DialogHeader>
              <div className="space-y-2">
                <Label htmlFor="agent_key_overlap">Overlap (hours)</Label>
                <Input
                  id="agent_key_overlap"
                  value={overlapHours}
                  onChange={(event) => setOverlapHours(event.target.value)}
                  inputMode="numeric"
                  aria-invalid={!overlapValid}
                />
                <p className="text-xs text-muted-foreground">
                  0 to {MAX_OVERLAP_HOURS}. Use 0 when the current secret has leaked.
                </p>
              </div>
              {rotateKey.error && (
                <Notice variant="destructive">
                  <NoticeDescription>{rotateKey.error.message}</NoticeDescription>
                </Notice>
              )}
              <DialogFooter>
                <Button type="button" variant="outline" onClick={closePending}>
                  Cancel
                </Button>
                <Button
                  type="button"
                  disabled={!overlapValid || rotateKey.isPending}
                  onClick={() =>
                    rotateKey.mutate(
                      { keyId: pending.key.id, overlapHours: overlap },
                      {
                        onSuccess: (rotated) => {
                          closePending();
                          setRevealed(rotated);
                        },
                      },
                    )
                  }
                >
                  {rotateKey.isPending ? "Rotating..." : "Rotate key"}
                </Button>
              </DialogFooter>
            </>
          )}
          {pending?.action === "revoke" && (
            <>
              <DialogHeader>
                <DialogTitle>Revoke {pending.key.name}</DialogTitle>
                <DialogDescription>
                  Both its current and previous secrets stop working at once. A revoked key cannot
                  be restored.
                </DialogDescription>
              </DialogHeader>
              {revokeKey.error && (
                <Notice variant="destructive">
                  <NoticeDescription>{revokeKey.error.message}</NoticeDescription>
                </Notice>
              )}
              <DialogFooter>
                <Button type="button" variant="outline" onClick={closePending}>
                  Cancel
                </Button>
                <Button
                  type="button"
                  variant="destructive"
                  disabled={revokeKey.isPending}
                  onClick={() => revokeKey.mutate(pending.key.id, { onSuccess: closePending })}
                >
                  {revokeKey.isPending ? "Revoking..." : "Revoke key"}
                </Button>
              </DialogFooter>
            </>
          )}
        </DialogContent>
      </Dialog>

      <Dialog open={revealed !== null} onOpenChange={(open) => !open && setRevealed(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Copy the secret for {revealed?.name}</DialogTitle>
            <DialogDescription>
              This is the only time it is shown. Store it in your application&apos;s secrets now.
            </DialogDescription>
          </DialogHeader>
          {revealed && (
            <div className="flex items-center gap-2 border bg-muted/40 p-3">
              <code className="min-w-0 flex-1 break-all text-sm" data-testid="agent-key-secret">
                {revealed.secret}
              </code>
              <CopyButton value={revealed.secret} label="Copy secret" />
            </div>
          )}
          <DialogFooter>
            <Button type="button" onClick={() => setRevealed(null)}>
              I stored it
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </Card>
  );
}
