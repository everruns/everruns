/**
 * New chat: pick a counterpart, then talk. The counterpart is chosen up front
 * because a thread is bound to it for its lifetime — afterwards the
 * binding is shown as a fact, not an editable control
 * (knowledge/ui/information-architecture.md).
 *
 * A thread is an ordinary session: this posts `POST /v1/sessions` with either
 * an agent or a harness binding. Direct harness chats let users start from a
 * configured runtime without creating an otherwise-empty agent first.
 *
 * An org with no model to chat with never reaches this form: both hosts own
 * their empty-state frame, so they swap the whole frame for the
 * "no intelligence available" message rather than stacking a second one here.
 */
"use client";

import { useState, type ReactNode } from "react";
import { useRouter } from "next/navigation";
import { Loader2, MessageCircle } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ChatErrorAlert } from "@/components/chat/chat-error-alert";
import { useAgents, useHarnesses } from "@/hooks";
import { useCreateSession } from "@/hooks/use-sessions";
import { CHAT_THREAD_TAG } from "@/lib/chat-threads";
import { getDisplayName } from "@/lib/entity-lifecycle";
import type { CreateSessionRequest } from "@/lib/api/types";

const AGENT_VALUE_PREFIX = "agent:";
const HARNESS_VALUE_PREFIX = "harness:";

export function NewChatForm({
  onStartingChange,
  surface = "chat",
  endUserId,
  children,
}: {
  surface?: "chat" | "playground";
  endUserId?: string;
  children?: ReactNode;
  /**
   * Fired when a thread starts being created, and again with `false` if it
   * fails. Creating a thread invalidates the session list, and a host that swaps
   * this form out on the first result would unmount it before the navigation
   * below runs — so the host holds the form in place until this says otherwise.
   */
  onStartingChange?: (starting: boolean) => void;
} = {}) {
  const router = useRouter();
  const { data: agents = [], isLoading: agentsLoading } = useAgents();
  const { data: allHarnesses = [], isLoading: harnessesLoading } = useHarnesses();
  const harnesses =
    surface === "playground"
      ? allHarnesses.filter((h) => !h.is_built_in || !h.name.startsWith("platform-chat"))
      : allHarnesses;
  const createSession = useCreateSession();
  const [selection, setSelection] = useState("");
  const [environment, setEnvironment] = useState("");
  const [error, setError] = useState<string | null>(null);
  const optionsLoading = agentsLoading || harnessesLoading;
  const selectedAgent = selection.startsWith(AGENT_VALUE_PREFIX)
    ? agents.find((agent) => agent.id === selection.slice(AGENT_VALUE_PREFIX.length))
    : undefined;
  const environmentProfiles = selectedAgent?.environments?.profiles ?? {};
  const environmentNames = Object.keys(environmentProfiles);

  const selectCounterpart = (value: string) => {
    setSelection(value);
    const agent = value.startsWith(AGENT_VALUE_PREFIX)
      ? agents.find((candidate) => candidate.id === value.slice(AGENT_VALUE_PREFIX.length))
      : undefined;
    setEnvironment(agent?.environments?.default ?? "");
  };

  const start = async () => {
    if (!selection) return;
    setError(null);
    onStartingChange?.(true);
    const binding: Partial<CreateSessionRequest> = selection.startsWith(HARNESS_VALUE_PREFIX)
      ? { harness_name: selection.slice(HARNESS_VALUE_PREFIX.length) }
      : { agent_id: selection.slice(AGENT_VALUE_PREFIX.length) };

    try {
      const session = await createSession.mutateAsync({
        request: {
          ...binding,
          ...(selectedAgent?.environments && environment
            ? { environment: { use: environment } }
            : {}),
          source: surface,
          ...(surface === "playground"
            ? { playground_user_id: endUserId }
            : { tags: [CHAT_THREAD_TAG] }),
        },
      });
      router.push(`/${surface === "playground" ? "playground" : "chats"}/${session.id}`);
    } catch (e) {
      onStartingChange?.(false);
      setError(e instanceof Error ? e.message : "Could not start the chat.");
    }
  };

  if (!optionsLoading && agents.length === 0 && harnesses.length === 0) {
    return (
      <div className="space-y-3">
        <p className="text-sm text-muted-foreground">
          There are no agents or harnesses available for a chat yet.
        </p>
        <Button onClick={() => router.push("/agents/new")}>Create an agent</Button>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <div
        className={
          surface === "playground"
            ? "flex flex-col gap-3"
            : "flex flex-wrap items-center justify-center gap-2"
        }
      >
        <Select value={selection} onValueChange={selectCounterpart} disabled={optionsLoading}>
          <SelectTrigger
            className={surface === "playground" ? "w-full" : "w-64"}
            aria-label="Chat counterpart"
          >
            <SelectValue
              placeholder={optionsLoading ? "Loading options..." : "Pick an agent or harness"}
            />
          </SelectTrigger>
          <SelectContent>
            {harnesses.length > 0 && (
              <SelectGroup>
                <SelectLabel>Harnesses</SelectLabel>
                {harnesses.map((harness) => (
                  <SelectItem key={harness.id} value={`${HARNESS_VALUE_PREFIX}${harness.name}`}>
                    {getDisplayName(harness)}
                  </SelectItem>
                ))}
              </SelectGroup>
            )}
            {agents.length > 0 && (
              <SelectGroup>
                <SelectLabel>Agents</SelectLabel>
                {agents.map((agent) => (
                  <SelectItem key={agent.id} value={`${AGENT_VALUE_PREFIX}${agent.id}`}>
                    {getDisplayName(agent)}
                  </SelectItem>
                ))}
              </SelectGroup>
            )}
          </SelectContent>
        </Select>
        {selectedAgent?.environments && environmentNames.length > 0 ? (
          <Select value={environment} onValueChange={setEnvironment}>
            <SelectTrigger
              className={surface === "playground" ? "w-full" : "w-56"}
              aria-label="Environment"
            >
              <SelectValue placeholder="Pick an environment" />
            </SelectTrigger>
            <SelectContent>
              <SelectGroup>
                <SelectLabel>Runs in</SelectLabel>
                {environmentNames.map((name) => {
                  const profile = environmentProfiles[name];
                  const target = profile.target.provider || profile.target.kind;
                  return (
                    <SelectItem key={name} value={name}>
                      {name === selectedAgent.environments?.default ? `${name} (default)` : name} ·{" "}
                      {target}
                    </SelectItem>
                  );
                })}
              </SelectGroup>
            </SelectContent>
          </Select>
        ) : null}
        {children}
        <Button
          onClick={start}
          disabled={
            !selection || createSession.isPending || (surface === "playground" && !endUserId)
          }
        >
          {createSession.isPending ? (
            <Loader2 className="size-4 animate-spin" />
          ) : (
            <MessageCircle className="size-4" />
          )}
          {surface === "playground" ? "Start conversation" : "Start chat"}
        </Button>
      </div>
      {error && <ChatErrorAlert message={error} />}
    </div>
  );
}
