"use client";

// Agents tab: the agents this agent hands work to.
//
// The list is the Agents capability's members. Each member is a real agent:
// "Created here" ones were made as part of this agent (they group under it on
// Agents home and travel in its export); "Linked" ones are any other agent in
// the organization. Names, labels and "Used when" come from the member agent
// itself, so they stay current. Removing a member never deletes the agent.

import { useMemo, useState } from "react";
import { useUiTranslations, useLocale as useDisplayLocale } from "@/providers/locale-provider";
import { useAgents, useCreateAgent, useModels, useUpdateAgent } from "@/hooks";
import type { Agent, AgentCapabilityConfig } from "@/lib/api/types";
import { getDisplayName } from "@/lib/entity-lifecycle";
import { initials } from "@/lib/agents-home";
import { AgentAvatar } from "@/components/agents/agent-avatar";
import { Badge } from "@/components/ui/badge";
import { Button, LinkButton } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Field } from "@/components/ui/field";
import { SegmentedControl } from "@/components/ui/segmented-control";
import {
  Drawer,
  DrawerContent,
  DrawerDescription,
  DrawerHeader,
  DrawerTitle,
} from "@/components/ui/drawer";

export const AGENTS_CAPABILITY = "agent_handoff";

export type Member = { agent_id: string } & Record<string, unknown>;

/** The members listed on an agent's Agents capability, or null without it. */
export function agentMembers(agent: Agent): Member[] | null {
  const cap = agent.capabilities.find((c) => c.ref === AGENTS_CAPABILITY);
  if (!cap) return null;
  const list = (cap.config?.targets ?? cap.config?.members ?? []) as Member[];
  return list.filter((m) => typeof m?.agent_id === "string");
}

/** Whether `candidate` already hands work to `agent`, so adding it would loop. */
function wouldLoop(candidate: Agent, agent: Agent): boolean {
  return (agentMembers(candidate) ?? []).some((m) => m.agent_id === agent.id);
}

/** The files a member starts with: its own, or a copy of this agent's session files. */
export function memberFiles(member: Member): "own" | "copy" {
  return member.files === "copy" ? "copy" : "own";
}

export function withFiles(member: Member, files: string): Member {
  const { files: _previous, ...rest } = member;
  return files === "copy"
    ? { ...rest, agent_id: member.agent_id, files: "copy" }
    : { ...rest, agent_id: member.agent_id };
}

function withMembers(agent: Agent, members: Member[]): AgentCapabilityConfig[] {
  return agent.capabilities.map((cap) =>
    cap.ref === AGENTS_CAPABILITY ? { ...cap, config: { ...cap.config, targets: members } } : cap,
  );
}

export function AgentMembersTab({ agent, canEdit }: { agent: Agent; canEdit: boolean }) {
  const ui = useUiTranslations();
  const { displayLocale } = useDisplayLocale();
  const { data: agents } = useAgents({});
  const { data: models } = useModels();
  const updateAgent = useUpdateAgent();
  const [adding, setAdding] = useState(false);
  const members = useMemo(() => agentMembers(agent) ?? [], [agent]);
  const byId = useMemo(() => new Map((agents ?? []).map((a) => [a.id, a])), [agents]);
  const modelName = (id: string | null | undefined) =>
    (models ?? []).find((m) => m.id === id)?.display_name ?? null;

  const save = (next: Member[], reason: string) =>
    updateAgent.mutateAsync({
      agentId: agent.id,
      request: { capabilities: withMembers(agent, next) },
      reason,
    });

  return (
    <section className="border bg-card">
      <header className="flex flex-wrap items-start justify-between gap-3 border-b px-5 py-4">
        <div>
          <h2 className="font-semibold">{ui("Agents")}</h2>
          <p className="text-sm text-muted-foreground">
            {ui(
              "This agent hands work to these agents. Each runs in its own conversation with its own model and tools.",
            )}
          </p>
        </div>
        {canEdit && <Button onClick={() => setAdding(true)}>{ui("Add agent")}</Button>}
      </header>
      {members.length === 0 ? (
        <p className="px-5 py-8 text-sm text-muted-foreground">
          {ui("No agents yet. Add an existing agent or create a new one here.")}
        </p>
      ) : (
        <ul className="grid gap-4 p-5 lg:grid-cols-2">
          {members.map((member) => {
            const target = byId.get(member.agent_id);
            const name = target
              ? getDisplayName(target, displayLocale)
              : String(member.name ?? member.id ?? member.agent_id);
            const createdHere = target?.owner_agent_id === agent.id;
            const model = modelName(target?.default_model_id);
            return (
              <li key={member.agent_id} className="flex flex-col gap-3 border p-4">
                <div className="flex items-start gap-3">
                  <AgentAvatar
                    avatar={target?.avatar}
                    size={36}
                    fallback={
                      <span className="inline-flex size-9 shrink-0 items-center justify-center bg-accent/25 text-xs font-semibold">
                        {initials(name)}
                      </span>
                    }
                  />
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="font-medium">{name}</span>
                      <Badge variant={createdHere ? "outline" : "accent"}>
                        {createdHere ? ui("Created here") : ui("Linked")}
                      </Badge>
                    </div>
                    <p className="text-[13px] text-muted-foreground">
                      {ui("Used when:")}{" "}
                      {target?.description || ui("No description. Add one on that agent.")}
                    </p>
                  </div>
                  <div className="flex shrink-0 gap-2">
                    {target && (
                      <LinkButton variant="outline" size="sm" href={`/agents/${target.id}`}>
                        {ui("Open")}
                      </LinkButton>
                    )}
                    {canEdit && (
                      <Button
                        variant="ghost"
                        size="sm"
                        disabled={updateAgent.isPending}
                        onClick={() =>
                          save(
                            members.filter((m) => m.agent_id !== member.agent_id),
                            `Stop handing work to ${name}`,
                          )
                        }
                      >
                        {ui("Remove")}
                      </Button>
                    )}
                  </div>
                </div>
                <dl className="flex flex-wrap gap-x-6 gap-y-1 text-[13px]">
                  <div className="flex gap-1">
                    <dt className="text-muted-foreground">{ui("Addressed as")}</dt>
                    <dd className="font-mono text-xs leading-5">{target?.name ?? "—"}</dd>
                  </div>
                  <div className="flex gap-1">
                    <dt className="text-muted-foreground">{ui("Model")}</dt>
                    <dd>{model ?? ui("Default")}</dd>
                  </div>
                  <div className="flex items-center gap-2">
                    <dt className="text-muted-foreground">{ui("Files")}</dt>
                    <dd>
                      {canEdit ? (
                        <SegmentedControl
                          label={ui("Files {agent} starts with", { agent: name })}
                          value={memberFiles(member)}
                          onValueChange={(files) =>
                            save(
                              members.map((m) =>
                                m.agent_id === member.agent_id ? withFiles(m, files) : m,
                              ),
                              files === "copy"
                                ? `Start ${name} with a copy of this agent's files`
                                : `Start ${name} with its own files`,
                            )
                          }
                          options={[
                            { value: "own", label: ui("Its own") },
                            { value: "copy", label: ui("A copy of this agent's") },
                          ]}
                        />
                      ) : memberFiles(member) === "copy" ? (
                        ui("A copy of this agent's")
                      ) : (
                        ui("Its own")
                      )}
                    </dd>
                  </div>
                  {!target && (
                    <div className="text-destructive">
                      {ui("This agent is no longer available.")}
                    </div>
                  )}
                </dl>
              </li>
            );
          })}
        </ul>
      )}
      {updateAgent.error && (
        <p role="alert" className="px-5 pb-4 text-sm text-destructive">
          {ui("Could not save:")} {updateAgent.error.message}
        </p>
      )}
      <AddAgentSheet
        open={adding}
        onOpenChange={setAdding}
        agent={agent}
        agents={agents ?? []}
        members={members}
        onAdd={async (added, reason) => {
          await save([...members, { agent_id: added.id }], reason);
          setAdding(false);
        }}
      />
    </section>
  );
}

function AddAgentSheet({
  open,
  onOpenChange,
  agent,
  agents,
  members,
  onAdd,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  agent: Agent;
  agents: Agent[];
  members: Member[];
  onAdd: (agent: Agent, reason: string) => Promise<void>;
}) {
  const ui = useUiTranslations();
  const { displayLocale } = useDisplayLocale();
  const createAgent = useCreateAgent();
  const [mode, setMode] = useState<"existing" | "new">("existing");
  const [search, setSearch] = useState("");
  const [picked, setPicked] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const listed = new Set(members.map((m) => m.agent_id));
  const candidates = agents.filter(
    (a) =>
      a.id !== agent.id &&
      !listed.has(a.id) &&
      a.status === "active" &&
      !a.is_built_in &&
      (!search.trim() ||
        `${getDisplayName(a, displayLocale)} ${a.name}`
          .toLowerCase()
          .includes(search.trim().toLowerCase())),
  );

  const submit = async () => {
    setError(null);
    setBusy(true);
    try {
      if (mode === "existing") {
        const target = agents.find((a) => a.id === picked);
        if (!target) return;
        await onAdd(target, `Hand work to ${target.name}`);
      } else {
        // A new agent starts from this agent's model and harness.
        const created = await createAgent.mutateAsync({
          name: name.trim(),
          description: description.trim() || undefined,
          system_prompt: description.trim() || "Do the work you are given and report back.",
          harness_id: agent.harness_id,
          default_model_id: agent.default_model_id ?? undefined,
          owner_agent_id: agent.id,
        });
        await onAdd(created, `Hand work to ${created.name}`);
      }
      setPicked(null);
      setName("");
      setDescription("");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Drawer open={open} onOpenChange={onOpenChange}>
      <DrawerContent className="sm:max-w-xl">
        <DrawerHeader className="p-0 pr-8">
          <DrawerTitle>{ui("Add agent")}</DrawerTitle>
          <DrawerDescription>
            {ui("Which agent should this agent be able to hand work to?")}
          </DrawerDescription>
        </DrawerHeader>
        <SegmentedControl
          label={ui("Add agent")}
          value={mode}
          onValueChange={(value) => setMode(value as "existing" | "new")}
          options={[
            { value: "existing", label: ui("Use an existing agent") },
            { value: "new", label: ui("Create a new one here") },
          ]}
        />
        {mode === "existing" ? (
          <div className="flex min-h-0 flex-1 flex-col gap-3">
            <Input
              placeholder={ui("Search agents…")}
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              aria-label={ui("Search agents…")}
            />
            <ul role="radiogroup" className="min-h-0 flex-1 space-y-2 overflow-y-auto">
              {candidates.map((candidate) => {
                const loop = wouldLoop(candidate, agent);
                const label = getDisplayName(candidate, displayLocale);
                return (
                  <li key={candidate.id}>
                    <button
                      type="button"
                      role="radio"
                      aria-checked={picked === candidate.id}
                      disabled={loop}
                      onClick={() => setPicked(candidate.id)}
                      className="flex w-full items-start gap-3 border p-3 text-left hover:bg-muted disabled:cursor-not-allowed disabled:opacity-50 aria-checked:border-foreground"
                    >
                      <span className="min-w-0">
                        <span className="block font-medium">{label}</span>
                        <span className="block text-[13px] text-muted-foreground">
                          {loop
                            ? ui("Can't add: it already hands work to this agent.")
                            : candidate.description || candidate.name}
                        </span>
                      </span>
                    </button>
                  </li>
                );
              })}
              {candidates.length === 0 && (
                <li className="text-sm text-muted-foreground">{ui("No agents to add.")}</li>
              )}
            </ul>
          </div>
        ) : (
          <div className="space-y-4">
            <Field
              id="member-name"
              label={ui("Name")}
              description={ui(
                "Lowercase letters, digits and hyphens. Other agents address it by this name.",
              )}
            >
              <Input
                id="member-name"
                value={name}
                onChange={(e) => setName(e.target.value.toLowerCase())}
                aria-describedby="member-name-hint"
              />
            </Field>
            <Field
              id="member-description"
              label={ui("When to use")}
              description={ui(
                "Becomes the new agent's description and starting instructions. It starts from this agent's model and harness, is listed under this agent, and is exported with it.",
              )}
            >
              <Textarea
                id="member-description"
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                aria-describedby="member-description-hint"
              />
            </Field>
          </div>
        )}
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
        <div className="flex justify-end gap-2">
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {ui("Cancel")}
          </Button>
          <Button
            disabled={
              busy ||
              (mode === "existing" ? !picked : !/^[a-z0-9]([a-z0-9-]*[a-z0-9])?$/.test(name.trim()))
            }
            onClick={submit}
          >
            {mode === "existing" ? ui("Add") : ui("Create and add")}
          </Button>
        </div>
      </DrawerContent>
    </Drawer>
  );
}
