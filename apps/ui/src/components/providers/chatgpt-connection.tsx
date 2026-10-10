"use client";
import { useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { ExternalLink, UserRound } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
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
import { useChatGptConnection } from "@/hooks/use-chatgpt-connection";
import {
  CHATGPT_USAGE_URL,
  disconnectChatGpt,
  importChatGptConnection,
  startChatGptLogin,
} from "@/lib/api/chatgpt";
import { queryKeys } from "@/lib/query-keys";

export function ChatGptConnectionCard({ providerId }: { providerId: string }) {
  const query = useChatGptConnection(providerId);
  const cache = useQueryClient();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState(false);
  const connection = query.data;
  const connected = connection?.status === "connected";
  const connecting = connection?.status === "connecting";
  const refresh = async () => {
    await cache.invalidateQueries({ queryKey: queryKeys.providers.all });
    await cache.invalidateQueries({ queryKey: queryKeys.models.all });
    await query.refetch();
  };
  useEffect(() => {
    if (connected) {
      setNotice(
        localStorage.getItem(
          `chatgpt-plan-notice:${connection?.owner_user_id ?? connection?.email ?? "account"}`,
        ) !== "seen",
      );
      void cache.invalidateQueries({ queryKey: queryKeys.models.all });
      void cache.invalidateQueries({ queryKey: queryKeys.providers.all });
    }
  }, [connected, connection?.owner_user_id, connection?.email, cache]);
  const act = async (action: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await action();
      await refresh();
    } catch {
      setError(
        "The connection could not be updated. Retry; if disconnect failed, your credentials were retained until revocation is confirmed.",
      );
    } finally {
      setBusy(false);
    }
  };
  const signIn = () => {
    // Open while the click still has user activation; navigation follows the API call.
    const popup = window.open("about:blank", "_blank");
    if (popup) popup.opener = null;
    void act(async () => {
      try {
        const { authorize_url } = await startChatGptLogin(providerId);
        if (popup) popup.location.href = authorize_url;
        else window.location.href = authorize_url;
      } catch (error) {
        popup?.close();
        throw error;
      }
    });
  };
  return (
    <>
      <Card>
        <CardHeader>
          <CardTitle>ChatGPT account</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex flex-wrap items-center gap-2">
            <UserRound className="size-4 text-muted-foreground" />
            <span className="text-sm font-medium">
              {connection?.email ?? "Your personal connection"}
            </span>
            <Badge variant="outline">Only you</Badge>
            <Badge variant={connected ? "success" : "outline"}>
              {connected ? "Connected" : connecting ? "Signing in…" : "Sign-in required"}
            </Badge>
          </div>
          <p className="max-w-xl text-sm text-muted-foreground">
            Runs use your ChatGPT plan and its usage limits. Organization sessions, other users, and
            Playground cannot use this account.
          </p>
          {(error || connection?.error || query.isError) && (
            <p role="alert" className="text-sm text-destructive">
              {error ?? connection?.error ?? "Could not load this connection."}
            </p>
          )}
          <div className="flex flex-wrap gap-2">
            <Button onClick={signIn} disabled={busy || connecting || query.isLoading}>
              {connecting
                ? "Waiting for ChatGPT…"
                : connected
                  ? "Reconnect"
                  : "Continue with ChatGPT"}
            </Button>
            <Button
              variant="outline"
              onClick={() => void act(() => disconnectChatGpt(providerId))}
              disabled={
                busy || (!connected && !connecting && connection?.status !== "scope_required")
              }
            >
              {connecting ? "Cancel sign-in" : "Disconnect"}
            </Button>
            <a
              href={CHATGPT_USAGE_URL}
              target="_blank"
              rel="noreferrer"
              className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
            >
              Manage usage <ExternalLink className="size-3.5" />
            </a>
          </div>
          <details className="border-t pt-4 text-sm">
            <summary className="cursor-pointer text-muted-foreground">
              Everruns runs on another computer?
            </summary>
            <div className="mt-3 max-w-xl space-y-3">
              <p className="text-muted-foreground">
                Run the local login helper on the computer with your browser, using this
                installation’s setup file. Upload its credential file here over your secure Everruns
                connection.
              </p>
              <div>
                <Label htmlFor="chatgpt-host-id">Host ID</Label>
                <Input
                  id="chatgpt-host-id"
                  className="mt-1 font-mono"
                  readOnly
                  value={connection?.host_id ?? ""}
                />
              </div>
              <Button
                variant="outline"
                disabled={!connection}
                onClick={() => {
                  const url = URL.createObjectURL(
                    new Blob(
                      [
                        JSON.stringify({
                          host_id: connection?.host_id,
                          registration: connection?.registration,
                        }),
                      ],
                      { type: "application/json" },
                    ),
                  );
                  const link = document.createElement("a");
                  link.href = url;
                  link.download = "everruns-chatgpt-setup.json";
                  link.click();
                  URL.revokeObjectURL(url);
                }}
              >
                Download login setup
              </Button>
              <a
                href="https://docs.everruns.com/features/chatgpt"
                target="_blank"
                rel="noreferrer"
                className="underline"
              >
                Local login helper instructions
              </a>
              <div>
                <Label htmlFor="chatgpt-import">Local login file</Label>
                <Input
                  id="chatgpt-import"
                  className="mt-1"
                  type="file"
                  accept="application/json,.json"
                  disabled={busy || connecting}
                  onChange={(event) => {
                    const file = event.target.files?.[0];
                    event.target.value = "";
                    if (!file) return;
                    void act(async () => {
                      if (file.size > 64 * 1024) throw new Error("Invalid login file");
                      await importChatGptConnection(providerId, JSON.parse(await file.text()));
                    });
                  }}
                />
              </div>
            </div>
          </details>
        </CardContent>
      </Card>
      <Dialog
        open={notice}
        onOpenChange={(open) => {
          if (!open) {
            localStorage.setItem(
              `chatgpt-plan-notice:${connection?.owner_user_id ?? connection?.email ?? "account"}`,
              "seen",
            );
            setNotice(false);
          }
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>You’re using your ChatGPT plan</DialogTitle>
            <DialogDescription>
              This personal connection uses your plan’s allowance. Check or manage usage in ChatGPT.
              You can switch to an API provider at any time.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              onClick={() => {
                localStorage.setItem(
                  `chatgpt-plan-notice:${connection?.owner_user_id ?? connection?.email ?? "account"}`,
                  "seen",
                );
                setNotice(false);
              }}
            >
              Got it
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
