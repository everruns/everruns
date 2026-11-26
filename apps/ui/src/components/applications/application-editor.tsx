"use client";
import { ApplicationRoomSettings } from "./application-room-settings";
import { useOrg } from "@/providers/org-provider";
import { useState } from "react";
import Link from "next/link";
import { useQueryClient } from "@tanstack/react-query";
import {
  PanelsTopLeft,
  ArrowUpRight,
  ArrowUp,
  ArrowDown,
  Plus,
  X,
  MessageSquare,
  Users,
  Database,
} from "lucide-react";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  SectionTabs,
} from "@/components/layout/page-layout";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Badge } from "@/components/ui/badge";
import { useUiTranslations } from "@/providers/locale-provider";
import { useAgents } from "@/hooks/use-agents";
import { useOrgScopedQuery } from "@/hooks/create-crud-hooks";
import {
  type Application,
  type ApplicationInput,
  type ApplicationResource,
  updateApplication,
  setApplicationStatus,
  createApplicationResource,
} from "@/lib/api/applications";
import { listAgentChannels } from "@/lib/api/agent-channels";
import { listRooms } from "@/lib/api/rooms";
import { listMemories } from "@/lib/api/memory";
import { usePageTitle } from "@/hooks";
const selectClass = "h-10 w-full rounded-md border border-input bg-background px-3 text-sm";
const slugify = (name: string) =>
  name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "")
    .slice(0, 48);
export function ApplicationEditor({
  application,
  onSaved,
}: {
  application: Application;
  onSaved: () => unknown;
}) {
  const ui = useUiTranslations();
  const { currentOrg } = useOrg();
  const canManage = currentOrg?.role === "owner" || currentOrg?.role === "admin";
  usePageTitle(application.name);
  const cache = useQueryClient();
  const [draft, setDraft] = useState<ApplicationInput>({
    name: application.name,
    slug: application.slug,
    description: application.description,
    config: application.config,
    resources: application.resources,
  });
  const [tab, setTab] = useState("resources"),
    [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [dirty, setDirty] = useState(false);
  const [kind, setKind] = useState<ApplicationResource["kind"]>("chat"),
    [name, setName] = useState(""),
    [agent, setAgent] = useState(""),
    [existing, setExisting] = useState(""),
    [mode, setMode] = useState("new");
  const agents = useAgents();
  const rooms = useOrgScopedQuery({
    queryKey: ["application-room-options"],
    queryFn: () => listRooms(),
  });
  const memories = useOrgScopedQuery({
    queryKey: ["application-memory-options"],
    queryFn: () => listMemories(),
  });
  const channels = useOrgScopedQuery({
    queryKey: ["application-channel-options", agent],
    queryFn: () => listAgentChannels(agent),
    enabled: !!agent,
  });
  function change(input: Partial<ApplicationInput>) {
    setDraft({ ...draft, ...input });
    setDirty(true);
  }
  async function act(fn: () => Promise<unknown>) {
    setBusy(true);
    setError("");
    try {
      await fn();
      await cache.invalidateQueries({ queryKey: ["applications"] });
      await cache.invalidateQueries({ queryKey: ["application-launchers"] });
      onSaved();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  async function add() {
    setBusy(true);
    setError("");
    try {
      let id = existing;
      let label = name.trim();
      if (mode === "new") {
        if (!label) throw new Error(ui("Enter a name"));
        if (kind === "chat" && !agent) throw new Error(ui("Select an agent"));
      } else {
        if (!id) throw new Error(ui("Select a resource"));
        label =
          label ||
          (kind === "chat"
            ? (agents.data ?? []).find((a) => a.id === agent)?.name
            : kind === "room"
              ? rooms.data?.find((r) => r.id === id)?.name
              : memories.data?.find((m) => m.id === id)?.name) ||
          ui("Resource");
      }
      let slug = slugify(label);
      if (!/^[a-z]/.test(slug)) slug = `resource-${slug}`;
      if (slug.length < 3) slug = `${slug}-resource`;
      const root = slug;
      let n = 2;
      while (draft.resources.some((r) => r.slug === slug)) slug = `${root.slice(0, 43)}-${n++}`;
      const next = {
        ...draft,
        resources: [
          ...draft.resources,
          { slug, name: label, kind, resource_id: id, writable: false },
        ],
      };
      if (mode === "new")
        await createApplicationResource(application.id, {
          version: application.version,
          kind,
          name: label,
          slug,
          agent_id: kind === "chat" ? agent : undefined,
        });
      else await updateApplication(application.id, next, application.version);
      setName("");
      setExisting("");
      setDirty(false);
      await cache.invalidateQueries({ queryKey: ["applications"] });
      onSaved();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  const options =
    kind === "chat"
      ? (channels.data ?? [])
          .filter((c) => c.channel_type === "work_chat")
          .map((c) => ({ id: c.id, name: c.id }))
      : kind === "room"
        ? (rooms.data ?? [])
        : (memories.data ?? []);
  const titles = { chat: ui("Chats"), room: ui("Rooms"), memory: ui("Memory") };
  return (
    <PageContainer>
      <PageBreadcrumb
        items={[{ label: ui("Applications"), href: "/applications" }, { label: application.name }]}
      />
      <PageMasthead
        icon={<PanelsTopLeft />}
        title={application.name}
        badges={
          <Badge variant="outline">
            {ui(
              application.status === "published"
                ? "Published"
                : application.status === "archived"
                  ? "Archived"
                  : "Draft",
            )}
          </Badge>
        }
        description={
          application.public_url ?? ui("Set up an Application domain to publish this application.")
        }
        actions={
          <div className="flex gap-2">
            {application.status === "published" && application.public_url && (
              <a
                href={application.public_url}
                target="_blank"
                rel="noreferrer"
                className="inline-flex items-center gap-2 text-sm"
              >
                {ui("Open application")}
                <ArrowUpRight className="size-4" />
              </a>
            )}
            <Button
              variant="outline"
              disabled={!canManage || application.status === "archived" || busy || !dirty}
              onClick={() =>
                void act(() => updateApplication(application.id, draft, application.version))
              }
            >
              {ui("Save changes")}
            </Button>
            <Button
              disabled={
                !canManage ||
                busy ||
                dirty ||
                (application.status !== "archived" && !draft.resources.length)
              }
              onClick={() =>
                void act(() =>
                  setApplicationStatus(
                    application.id,
                    application.status === "published" || application.status === "archived"
                      ? "draft"
                      : "published",
                    application.version,
                  ),
                )
              }
            >
              {ui(
                application.status === "published"
                  ? "Unpublish"
                  : application.status === "archived"
                    ? "Restore"
                    : "Publish",
              )}
            </Button>
          </div>
        }
      />
      {error && (
        <p className="mb-4 text-destructive" role="alert">
          {error}
        </p>
      )}
      {dirty && (
        <p className="mb-3 text-sm text-muted-foreground">
          {ui("Save your changes before publishing.")}
        </p>
      )}
      <SectionTabs
        value={tab}
        onValueChange={setTab}
        items={[
          { value: "resources", label: ui("Resources") },
          { value: "audience", label: ui("Audience") },
          { value: "settings", label: ui("Settings") },
        ]}
      />
      <fieldset
        disabled={!canManage || application.status === "archived"}
        className="min-w-0 border-0 p-0"
      >
        {tab === "resources" && (
          <div className="grid gap-8 pt-6 xl:grid-cols-[1fr_340px]">
            <div className="space-y-6">
              {(["chat", "room", "memory"] as const).map((kind) => (
                <section key={kind}>
                  <h2 className="mb-3 flex items-center gap-2 text-sm font-semibold">
                    {kind === "chat" ? (
                      <MessageSquare className="size-4" />
                    ) : kind === "room" ? (
                      <Users className="size-4" />
                    ) : (
                      <Database className="size-4" />
                    )}
                    {titles[kind]}
                  </h2>
                  {!draft.resources.some((r) => r.kind === kind) && (
                    <p className="border border-dashed border-border p-5 text-sm text-muted-foreground">
                      {ui("Add a resource to include it in your application.")}
                    </p>
                  )}
                  {draft.resources.map((r, index) =>
                    r.kind !== kind ? null : (
                      <div
                        key={r.resource_id}
                        className="mb-2 grid gap-3 border border-border bg-card p-4"
                      >
                        <div className="flex items-center gap-2">
                          <Input
                            aria-label={ui("Resource name")}
                            value={r.name}
                            onChange={(e) =>
                              change({
                                resources: draft.resources.map((item, i) =>
                                  i === index ? { ...item, name: e.target.value } : item,
                                ),
                              })
                            }
                          />
                          <Button
                            variant="ghost"
                            size="icon"
                            aria-label={ui("Move up")}
                            disabled={index === 0}
                            onClick={() => {
                              const resources = [...draft.resources];
                              [resources[index - 1], resources[index]] = [
                                resources[index],
                                resources[index - 1],
                              ];
                              change({ resources });
                            }}
                          >
                            <ArrowUp className="size-4" />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon"
                            aria-label={ui("Move down")}
                            disabled={index === draft.resources.length - 1}
                            onClick={() => {
                              const resources = [...draft.resources];
                              [resources[index + 1], resources[index]] = [
                                resources[index],
                                resources[index + 1],
                              ];
                              change({ resources });
                            }}
                          >
                            <ArrowDown className="size-4" />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon"
                            aria-label={ui("Remove resource")}
                            onClick={() =>
                              change({ resources: draft.resources.filter((_, i) => i !== index) })
                            }
                          >
                            <X className="size-4" />
                          </Button>
                        </div>
                        <label className="flex items-center gap-2 text-xs text-muted-foreground">
                          {ui("Path")}
                          <Input
                            value={r.slug}
                            aria-label={ui("Resource path")}
                            onChange={(e) =>
                              change({
                                resources: draft.resources.map((item, i) =>
                                  i === index ? { ...item, slug: e.target.value } : item,
                                ),
                              })
                            }
                          />
                        </label>
                        {kind === "memory" && (
                          <label className="flex items-center gap-2 text-sm">
                            <input
                              type="checkbox"
                              checked={r.writable}
                              onChange={(e) =>
                                change({
                                  resources: draft.resources.map((item, i) =>
                                    i === index ? { ...item, writable: e.target.checked } : item,
                                  ),
                                })
                              }
                            />
                            {ui("Allow people and agents to edit this memory")}
                          </label>
                        )}
                        {kind === "room" && <ApplicationRoomSettings roomId={r.resource_id} />}
                        <div className="flex justify-between text-xs text-muted-foreground">
                          <code className="truncate">{r.resource_id}</code>
                          {kind !== "chat" && (
                            <Link
                              href={`/${kind === "room" ? "rooms" : "memory"}/${r.resource_id}`}
                            >
                              {ui("Manage resource")} ↗
                            </Link>
                          )}
                        </div>
                      </div>
                    ),
                  )}
                </section>
              ))}
              <section className="border border-dashed border-border p-5">
                <h2 className="text-sm font-semibold">{ui("Automations")}</h2>
                <p className="mt-2 text-sm text-muted-foreground">
                  {ui("Automations will be available when the automation runtime ships.")}
                </p>
              </section>
            </div>
            <aside className="h-fit space-y-4 border border-border bg-card p-5">
              <h2 className="font-semibold">{ui("Add a resource")}</h2>
              <label className="grid gap-2 text-sm">
                {ui("Type")}
                <select
                  className={selectClass}
                  value={kind}
                  onChange={(e) => {
                    setKind(e.target.value as ApplicationResource["kind"]);
                    setExisting("");
                  }}
                >
                  {Object.entries(titles).map(([value, label]) => (
                    <option key={value} value={value}>
                      {label}
                    </option>
                  ))}
                </select>
              </label>
              <select
                aria-label={ui("Resource source")}
                className={selectClass}
                value={mode}
                onChange={(e) => setMode(e.target.value)}
              >
                <option value="new">{ui("Create new")}</option>
                <option value="existing">{ui("Use existing")}</option>
              </select>
              {kind === "chat" && (
                <label className="grid gap-2 text-sm">
                  {ui("Agent")}
                  <select
                    className={selectClass}
                    value={agent}
                    onChange={(e) => {
                      setAgent(e.target.value);
                      setExisting("");
                    }}
                  >
                    <option value="">{ui("Select an agent")}</option>
                    {agents.data?.map((a) => (
                      <option key={a.id} value={a.id}>
                        {a.name}
                      </option>
                    ))}
                  </select>
                </label>
              )}
              {mode === "existing" && (
                <label className="grid gap-2 text-sm">
                  {ui("Resource")}
                  <select
                    className={selectClass}
                    value={existing}
                    onChange={(e) => setExisting(e.target.value)}
                  >
                    <option value="">{ui("Select a resource")}</option>
                    {options
                      .filter((r) => !draft.resources.some((d) => d.resource_id === r.id))
                      .map((r) => (
                        <option key={r.id} value={r.id}>
                          {r.name}
                        </option>
                      ))}
                  </select>
                </label>
              )}
              <label className="grid gap-2 text-sm">
                {ui("Display name")}
                <Input value={name} maxLength={120} onChange={(e) => setName(e.target.value)} />
              </label>
              <Button disabled={busy || dirty} onClick={() => void add()}>
                <Plus className="mr-2 size-4" />
                {ui("Add resource")}
              </Button>
              <p className="text-xs leading-relaxed text-muted-foreground">
                {ui(
                  "Chats and rooms belong to one application audience. Attaching an existing resource makes its content available to that audience.",
                )}
              </p>
            </aside>
          </div>
        )}
        {tab === "audience" && (
          <section className="max-w-2xl space-y-6 py-6">
            <div>
              <h2 className="font-semibold">{ui("One audience for the whole application")}</h2>
              <p className="mt-2 text-sm text-muted-foreground">
                {ui(
                  "People sign in through the deployment’s identity provider. Access is checked on every request.",
                )}
              </p>
            </div>
            {(
              [
                [
                  "org_users",
                  "Organization users",
                  "Use existing organization accounts and their connected services.",
                ],
                [
                  "invites",
                  "Invited people",
                  "Only people with an invited, verified email address can sign in.",
                ],
                [
                  "open",
                  "Anyone who signs in",
                  "A separate application identity for anyone accepted by the identity provider.",
                ],
              ] as const
            ).map(([value, label, description]) => (
              <label
                key={value}
                className="flex cursor-pointer items-start gap-3 border border-border p-4"
              >
                <input
                  className="mt-1"
                  type="radio"
                  name="audience"
                  checked={draft.config.membership === value}
                  onChange={() => change({ config: { ...draft.config, membership: value } })}
                />
                <span>
                  <strong className="text-sm">{ui(label)}</strong>
                  <span className="mt-1 block text-sm text-muted-foreground">
                    {ui(description)}
                  </span>
                </span>
              </label>
            ))}
            {draft.config.membership === "invites" && (
              <label className="grid gap-2 text-sm">
                {ui("Invited email addresses")}
                <Textarea
                  rows={7}
                  value={draft.config.invites.join("\n")}
                  onChange={(e) =>
                    change({ config: { ...draft.config, invites: e.target.value.split("\n") } })
                  }
                />
                <span className="text-xs text-muted-foreground">
                  {ui("One address per line, up to 50 people.")}
                </span>
              </label>
            )}
          </section>
        )}
        {tab === "settings" && (
          <section className="max-w-2xl space-y-6 py-6">
            <label className="grid gap-2 text-sm">
              {ui("Name")}
              <Input
                maxLength={120}
                value={draft.name}
                onChange={(e) => change({ name: e.target.value })}
              />
            </label>
            <label className="grid gap-2 text-sm">
              {ui("Description")}
              <Textarea
                rows={3}
                maxLength={2000}
                value={draft.description}
                onChange={(e) => change({ description: e.target.value })}
              />
            </label>
            <label className="grid gap-2 text-sm">
              {ui("Stable name")}
              <Input value={draft.slug} readOnly />
              <span className="text-xs text-muted-foreground">
                {ui("Used in the public address. This name stays reserved and cannot be changed.")}
              </span>
            </label>
            <label className="flex items-center gap-3 text-sm">
              <input
                type="checkbox"
                checked={draft.config.show_in_sidebar}
                onChange={(e) =>
                  change({ config: { ...draft.config, show_in_sidebar: e.target.checked } })
                }
              />
              {ui("Show a launcher in the Everruns sidebar")}
            </label>
            <p className="text-sm text-muted-foreground">
              {ui(
                "Sidebar visibility does not grant access. The application’s audience still applies.",
              )}
            </p>
            <div className="border-t border-border pt-6">
              <h2 className="font-semibold">{ui("Archive application")}</h2>
              <p className="my-2 text-sm text-muted-foreground">
                {ui(
                  "Stops public access and keeps its name reserved. Resources and conversation history are retained.",
                )}
              </p>
              <Button
                variant="destructive"
                disabled={busy || application.status === "archived"}
                onClick={() => {
                  if (window.confirm(ui("Archive this application and stop public access?")))
                    void act(() =>
                      setApplicationStatus(application.id, "archived", application.version),
                    );
                }}
              >
                {ui("Archive application")}
              </Button>
            </div>
          </section>
        )}
      </fieldset>
    </PageContainer>
  );
}
