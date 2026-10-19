"use client";
import { useState } from "react";
import { KeyRound } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import { ScopedCredentialsPanel } from "./scoped-credentials-panel";

export function SessionCredentialsDialog({ sessionId }: { sessionId: string }) {
  const [open, setOpen] = useState(false);
  const [usage, setUsage] = useState<"user" | "service">("user");
  return (
    <>
      <Button variant="outline" size="sm" onClick={() => setOpen(true)}>
        <KeyRound className="size-4" />
        Credentials
      </Button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="max-h-[85vh] max-w-3xl overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Session credentials</DialogTitle>
            <DialogDescription>
              Choose which identity may use the credentials. Personal credentials remain private to
              the verified person.
            </DialogDescription>
          </DialogHeader>
          <div className="flex gap-2" role="group" aria-label="Credential acting identity">
            <Button
              size="sm"
              aria-pressed={usage === "user"}
              variant={usage === "user" ? "secondary" : "outline"}
              onClick={() => setUsage("user")}
            >
              Personal
            </Button>
            <Button
              size="sm"
              aria-pressed={usage === "service"}
              variant={usage === "service" ? "secondary" : "outline"}
              onClick={() => setUsage("service")}
            >
              Session service
            </Button>
          </div>
          {open && (
            <ScopedCredentialsPanel
              key={`${sessionId}:${usage}`}
              owner={{ scope: "session", owner_id: sessionId, usage }}
            />
          )}
        </DialogContent>
      </Dialog>
    </>
  );
}
