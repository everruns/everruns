"use client";

// New Playground room: pick the agents, and optionally test users to post as.
// A Playground room is an ordinary room marked `metadata.playground`, made
// through the same API an app calls.

import { useRouter } from "next/navigation";
import { useId, useState } from "react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Field } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { ResourceMultiSelect } from "@/components/ui/resource-multi-select";
import { useAgents, useVirtualUsers } from "@/hooks";
import { useCreateRoom } from "@/hooks/use-rooms";
import type { RoomMemberInput } from "@/lib/api/rooms";
import { getDisplayName } from "@/lib/entity-lifecycle";
import { PLAYGROUND_METADATA_KEY, guestDisplayName } from "@/lib/rooms";
import { cn } from "@/lib/utils";
import { useLocale as useDisplayLocale, useUiTranslations } from "@/providers/locale-provider";

export function NewRoomDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const ui = useUiTranslations();
  const { displayLocale } = useDisplayLocale();
  const router = useRouter();
  const id = useId();
  const { data: agents = [] } = useAgents();
  const { data: virtualUsers = [] } = useVirtualUsers({ usage: "end_user", enabled: open });
  const createRoom = useCreateRoom();
  const [name, setName] = useState("");
  const [agentIds, setAgentIds] = useState<string[]>([]);
  const [people, setPeople] = useState<"test-users" | "only-you">("test-users");
  const [testUserIds, setTestUserIds] = useState<string[]>([]);

  const create = async () => {
    const members: RoomMemberInput[] = agentIds.map((agentId) => ({
      kind: "agent",
      agent_id: agentId,
    }));
    if (people === "test-users") {
      for (const userId of testUserIds) {
        const user = virtualUsers.find((candidate) => candidate.id === userId);
        if (user) members.push({ kind: "guest", display_name: guestDisplayName(user.name) });
      }
    }
    const room = await createRoom.mutateAsync({
      name: name.trim() || ui("Playground room"),
      metadata: { [PLAYGROUND_METADATA_KEY]: true },
      members,
    });
    onOpenChange(false);
    router.push(`/playground/rooms/${room.id}`);
  };

  const option = (value: typeof people, title: string, description?: string) => (
    <label
      className={cn(
        "flex cursor-pointer items-start gap-3 border px-3 py-2.5",
        people === value ? "border-foreground bg-muted/50" : "border-border",
      )}
    >
      <input
        type="radio"
        name={`${id}-people`}
        className="mt-1"
        checked={people === value}
        onChange={() => setPeople(value)}
      />
      <span>
        <span className="block text-sm font-medium">{title}</span>
        {description && <span className="block text-xs text-muted-foreground">{description}</span>}
      </span>
    </label>
  );

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        chatProtected="New room"
        className="max-h-[calc(100svh-2rem)] max-w-xl overflow-y-auto"
      >
        <DialogHeader>
          <DialogTitle>{ui("New Playground room")}</DialogTitle>
          <DialogDescription>
            {ui(
              "Try how your agents behave together in a room before an app uses them. It is the same room API your apps call.",
            )}
          </DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-4">
          <Field id={`${id}-name`} label={ui("Name")}>
            <Input
              id={`${id}-name`}
              value={name}
              placeholder={ui("Launch planning test")}
              onChange={(event) => setName(event.target.value)}
            />
          </Field>
          <ResourceMultiSelect
            label={ui("Agents in the room")}
            options={agents.map((agent) => ({
              value: agent.id,
              label: getDisplayName(agent, displayLocale),
            }))}
            value={agentIds}
            onChange={setAgentIds}
            emptyMessage={ui("No agents yet")}
          />
          <p className="-mt-2 text-xs text-muted-foreground">
            {ui(
              "An agent wakes when it is mentioned, and on replies in threads it is in. Change this per agent in the room.",
            )}
          </p>
          <fieldset className="flex flex-col gap-2">
            <legend className="mb-2 text-sm font-medium">{ui("People in the room")}</legend>
            {option(
              "test-users",
              ui("You, and test users you post as"),
              ui(
                "Post as test users named after your virtual users, to try a conversation with several people.",
              ),
            )}
            {option("only-you", ui("Only you"))}
          </fieldset>
          {people === "test-users" && (
            <ResourceMultiSelect
              label={ui("Test users")}
              options={virtualUsers.map((user) => ({ value: user.id, label: user.name }))}
              value={testUserIds}
              onChange={setTestUserIds}
              emptyMessage={ui("No virtual users yet")}
            />
          )}
          <div>
            <div className="text-sm font-medium">{ui("Limits")}</div>
            <p className="text-xs text-muted-foreground">
              {ui(
                "6 agent replies in a row · 30 wakes per agent an hour · 200 agent messages an hour",
              )}
            </p>
          </div>
          {createRoom.error && (
            <p role="alert" className="text-sm text-destructive">
              {createRoom.error.message}
            </p>
          )}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {ui("Cancel")}
          </Button>
          <Button
            onClick={() => void create()}
            loading={createRoom.isPending}
            disabled={agentIds.length === 0}
          >
            {ui("Create room")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
