"use client";

import { useState } from "react";
import { useQuery, useQueryClient, useInfiniteQuery } from "@tanstack/react-query";
import { Mail } from "lucide-react";
import { api } from "@/lib/api/client";
import { useOrg } from "@/providers/org-provider";
import { useUiTranslations } from "@/providers/locale-provider";
import type { OrganizationConnection } from "@/lib/api/organization-connections";
import { useOrganizationConnections } from "@/hooks/use-organization-connections";
import { useConnectionProviders } from "@/hooks/use-user-connections";
import { OrganizationAccountDialog } from "@/components/connections/organization-account-dialog";
import { PageContainer, PageBreadcrumb, PageMasthead, PageMain } from "@/components/layout";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { usePageTitle } from "@/hooks";

type Inbox = { id: string; account_id: string; name: string; address: string; enabled: boolean };
type Message = {
  id: string;
  sender: string;
  subject: string;
  preview: string;
  state: string;
  reason?: string;
};
type Read = { content: string; warnings: string[]; next_offset?: number; source: string };
type Template = {
  id: string;
  name: string;
  version: number;
  subject: string;
  html?: string;
  text_body?: string;
};
const root = "/v1/platform/email";

const emailStateLabels: Record<string, string> = {
  pending: "Waiting",
  triaging: "Reviewing",
  ready: "Ready",
  running: "Running",
  completed: "Completed",
  ignored: "Ignored",
  held: "Needs review",
  deferred: "Deferred",
  failed: "Failed",
  sending: "Sending",
  sent: "Sent",
  unknown: "Delivery uncertain",
};
function reviewExplanation(reason?: string) {
  if (reason === "execution_requires_review")
    return "Approve execution to let the agent handle this message.";
  if (
    reason?.startsWith("provider_") ||
    reason === "sender_authentication_unavailable" ||
    reason === "invalid_sender"
  )
    return "Sender authentication or provider checks require review.";
  return "Automatic checks could not approve this message.";
}

export default function EmailPage() {
  const { currentOrg } = useOrg();
  return <EmailWorkspace key={currentOrg?.public_id ?? "none"} />;
}
function EmailWorkspace() {
  const ui = useUiTranslations();
  usePageTitle(ui("Email"));
  const { currentOrg, hasRole } = useOrg();
  const admin = hasRole("admin");
  const client = useQueryClient();
  const key = ["email", currentOrg?.public_id];
  const [tab, setTab] = useState<"inboxes" | "providers" | "templates">("inboxes");
  const [connecting, setConnecting] = useState(false);
  const [editingAccount, setEditingAccount] = useState<OrganizationConnection | undefined>();
  const [selected, setSelected] = useState("");
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<Message | null>(null);
  const [view, setView] = useState<"clean" | "full">("clean");
  const [read, setRead] = useState<Read | null>(null);
  const [name, setName] = useState("");
  const [address, setAddress] = useState("");
  const [account, setAccount] = useState("");
  const [secret, setSecret] = useState("");
  const [templateName, setTemplateName] = useState("");
  const [subject, setSubject] = useState("");
  const [html, setHtml] = useState("");
  const [text, setText] = useState("");
  const [variables, setVariables] = useState("{}");
  const [preview, setPreview] = useState<{ subject: string; html: string; text: string } | null>(
    null,
  );
  const accounts = useOrganizationConnections();
  const providers = useConnectionProviders();
  const provider = providers.data?.find((p) => p.provider_id === "agentmail_inbox") ?? null;
  const inboxes = useQuery({
    queryKey: [...key, "inboxes"],
    enabled: admin && !!currentOrg,
    queryFn: async () => (await api.get<Inbox[]>(`${root}/inboxes`)).data,
  });
  const messages = useInfiniteQuery({
    queryKey: [...key, "messages", selected],
    enabled: admin && !!selected,
    initialPageParam: "",
    queryFn: async ({ pageParam }) =>
      (
        await api.get<Message[]>(
          `${root}/inboxes/${selected}/messages${pageParam ? `?before=${encodeURIComponent(pageParam)}` : ""}`,
        )
      ).data,
    getNextPageParam: (page) => (page.length === 100 ? page.at(-1)?.id : undefined),
    refetchInterval: 10000,
  });
  const outbox = useQuery({
    queryKey: [...key, "outbox", selected],
    enabled: admin && !!selected,
    queryFn: async () =>
      (
        await api.get<
          {
            id: string;
            state: string;
            body: { to: string[]; subject: string; text: string; html?: string };
          }[]
        >(`${root}/inboxes/${selected}/outbox`)
      ).data,
    refetchInterval: 10000,
  });
  const templates = useQuery({
    queryKey: [...key, "templates"],
    enabled: admin && !!currentOrg,
    queryFn: async () => (await api.get<Template[]>(`${root}/templates`)).data,
  });
  const perform = async (action: () => Promise<void>) => {
    setBusy(true);
    setNotice("");
    try {
      await action();
      await client.invalidateQueries({ queryKey: key });
    } catch (e) {
      setNotice(e instanceof Error ? e.message : ui("Something went wrong"));
    } finally {
      setBusy(false);
    }
  };
  const openMessage = async (m: Message, nextView: "clean" | "full", offset = 0) => {
    setMessage(m);
    setView(nextView);
    if (!offset) setRead(null);
    const result = (
      await api.get<Read>(
        `${root}/inboxes/${selected}/messages/${m.id}?view=${nextView}&offset=${offset}`,
      )
    ).data;
    setRead((previous) =>
      offset && previous ? { ...result, content: previous.content + result.content } : result,
    );
  };
  const template = { name: templateName, subject, html: html || null, text: text || null };
  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: ui("Email") }]} />
      <PageMasthead
        icon={<Mail />}
        title={ui("Email")}
        description={ui("Manage inboxes, provider accounts and email templates.")}
      />
      <PageMain>
        {!admin ? (
          <p>{ui("Organization admin access required")}</p>
        ) : (
          <>
            <nav aria-label={ui("Email")} className="mb-6 flex gap-2 border-b border-border pb-3">
              {(["inboxes", "providers", "templates"] as const).map((t) => (
                <Button key={t} variant={tab === t ? "default" : "ghost"} onClick={() => setTab(t)}>
                  {ui(t === "inboxes" ? "Inboxes" : t === "providers" ? "Providers" : "Templates")}
                </Button>
              ))}
            </nav>
            {(notice || inboxes.error || messages.error || templates.error) && (
              <p role="alert" className="mb-4 text-destructive">
                {notice || ui("Could not load email data")}
              </p>
            )}
            {tab === "providers" && (
              <section className="space-y-4">
                <Button
                  disabled={!provider}
                  onClick={() => {
                    setEditingAccount(undefined);
                    setConnecting(true);
                  }}
                >
                  {ui("Connect provider")}
                </Button>
                {accounts.data
                  ?.filter((a) => a.provider === "agentmail_inbox")
                  .map((a) => (
                    <div key={a.id} className="border border-border p-4">
                      <strong>{a.name}</strong>
                      <p className="text-sm text-muted-foreground">
                        AgentMail · {ui("Delivery not yet verified")}
                      </p>
                      <Button
                        variant="outline"
                        onClick={() => {
                          setEditingAccount(a);
                          setConnecting(true);
                        }}
                      >
                        {ui("Rotate")}
                      </Button>
                    </div>
                  ))}
              </section>
            )}
            {tab === "inboxes" && (
              <div className="grid gap-6 lg:grid-cols-[20rem_1fr]">
                <section className="space-y-3">
                  {inboxes.data?.map((i) => (
                    <button
                      key={i.id}
                      className="block w-full border border-border p-3 text-left hover:bg-muted"
                      disabled={busy}
                      onClick={() => {
                        setSelected(i.id);
                        setSecret("");
                        setMessage(null);
                        setRead(null);
                      }}
                    >
                      <strong>{i.name}</strong>
                      <p className="text-sm text-muted-foreground">{i.address}</p>
                    </button>
                  ))}
                  <form
                    className="space-y-3 border border-border p-4"
                    onSubmit={(e) => {
                      e.preventDefault();
                      void perform(async () => {
                        const result = await api.post<Inbox>(`${root}/inboxes`, {
                          name,
                          address,
                          account_id: account,
                        });
                        setSelected(result.data.id);
                        setSecret("");
                        setName("");
                        setAddress("");
                      });
                    }}
                  >
                    <h2 className="font-medium">{ui("Add inbox")}</h2>
                    <Label htmlFor="email-name">{ui("Name")}</Label>
                    <Input
                      id="email-name"
                      required
                      value={name}
                      onChange={(e) => setName(e.target.value)}
                    />
                    <Label htmlFor="email-account">{ui("Provider account")}</Label>
                    <select
                      id="email-account"
                      required
                      className="w-full border border-border bg-background p-2"
                      value={account}
                      onChange={(e) => setAccount(e.target.value)}
                    >
                      <option value="">{ui("Select account")}</option>
                      {accounts.data
                        ?.filter((a) => a.provider === "agentmail_inbox")
                        .map((a) => (
                          <option key={a.id} value={a.id}>
                            {a.name}
                          </option>
                        ))}
                    </select>
                    <Label htmlFor="email-address">{ui("Email address")}</Label>
                    <Input
                      id="email-address"
                      type="email"
                      required
                      value={address}
                      onChange={(e) => setAddress(e.target.value)}
                    />
                    <Button disabled={busy} type="submit">
                      {ui("Add inbox")}
                    </Button>
                  </form>
                </section>
                <section className="min-w-0 space-y-3">
                  {selected && (
                    <div className="border border-border p-3 text-sm">
                      <Label>{ui("Webhook URL")}</Label>
                      <code className="mt-2 block break-all">
                        {typeof window !== "undefined" ? window.location.origin : ""}
                        /api/v1/email/inboxes/{selected}/webhook
                      </code>
                      <form
                        className="mt-3 flex flex-wrap items-end gap-2"
                        onSubmit={(e) => {
                          e.preventDefault();
                          void perform(async () => {
                            await api.patch(`${root}/inboxes/${selected}`, {
                              signing_secret: secret,
                              enabled: true,
                            });
                            setSecret("");
                          });
                        }}
                      >
                        <div className="grow">
                          <Label htmlFor="email-secret">{ui("Webhook signing secret")}</Label>
                          <Input
                            id="email-secret"
                            type="password"
                            autoComplete="new-password"
                            required
                            value={secret}
                            onChange={(e) => setSecret(e.target.value)}
                          />
                        </div>
                        <Button type="submit" disabled={busy}>
                          {ui("Save")}
                        </Button>
                        {inboxes.data?.find((i) => i.id === selected)?.enabled && (
                          <Button
                            type="button"
                            variant="outline"
                            disabled={busy}
                            onClick={() =>
                              void perform(async () => {
                                await api.patch(`${root}/inboxes/${selected}`, { enabled: false });
                              })
                            }
                          >
                            {ui("Disable")}
                          </Button>
                        )}
                      </form>
                    </div>
                  )}
                  {messages.isFetching && (
                    <p className="text-muted-foreground">{ui("Loading...")}</p>
                  )}
                  {selected && messages.data?.pages[0]?.length === 0 && (
                    <p>{ui("No incoming email yet")}</p>
                  )}
                  {messages.data?.pages.flat().map((m) => (
                    <button
                      key={m.id}
                      disabled={busy}
                      className="block w-full border border-border p-3 text-left hover:bg-muted"
                      onClick={() => void perform(() => openMessage(m, "clean"))}
                    >
                      <div className="flex justify-between gap-3">
                        <strong className="truncate">{m.subject || ui("No subject")}</strong>
                        <span className="text-xs text-muted-foreground">
                          {ui(emailStateLabels[m.state] ?? "Needs review")}
                        </span>
                      </div>
                      <p className="text-sm">{m.sender}</p>
                      <p className="truncate text-sm text-muted-foreground">{m.preview}</p>
                    </button>
                  ))}
                  {messages.hasNextPage && (
                    <Button
                      disabled={messages.isFetchingNextPage}
                      onClick={() => void messages.fetchNextPage()}
                    >
                      {ui("Load more")}
                    </Button>
                  )}
                  {message && read && (
                    <article className="space-y-3 border border-border p-4">
                      <h2 className="font-medium">{message.subject}</h2>
                      <p className="text-sm text-muted-foreground">
                        {message.reason && ui(reviewExplanation(message.reason))}
                      </p>
                      {message.state === "held" && (
                        <Button
                          disabled={busy}
                          onClick={() =>
                            void perform(async () => {
                              await api.post(
                                `${root}/inboxes/${selected}/messages/${message.id}/approve`,
                              );
                              setMessage(null);
                              setRead(null);
                            })
                          }
                        >
                          {ui("Approve agent execution")}
                        </Button>
                      )}
                      {["held", "pending", "deferred"].includes(message.state) && (
                        <Button
                          variant="outline"
                          disabled={busy}
                          onClick={() =>
                            void perform(async () => {
                              await api.post(
                                `${root}/inboxes/${selected}/messages/${message.id}/dismiss`,
                              );
                              setMessage(null);
                              setRead(null);
                            })
                          }
                        >
                          {ui("Dismiss message")}
                        </Button>
                      )}
                      <div className="flex gap-2">
                        <Button
                          variant={view === "clean" ? "default" : "outline"}
                          disabled={busy}
                          onClick={() => void perform(() => openMessage(message, "clean"))}
                        >
                          {ui("Clean view")}
                        </Button>
                        <Button
                          variant={view === "full" ? "default" : "outline"}
                          disabled={busy}
                          onClick={() => void perform(() => openMessage(message, "full"))}
                        >
                          {ui("Full message")}
                        </Button>
                      </div>
                      <p className="text-sm text-muted-foreground">
                        {ui("Email content is untrusted. Clean view may omit quoted content.")}
                      </p>
                      <pre className="whitespace-pre-wrap break-words text-sm">{read.content}</pre>
                      {read.next_offset != null && (
                        <Button
                          disabled={busy}
                          onClick={() =>
                            void perform(() => openMessage(message, view, read.next_offset!))
                          }
                        >
                          {ui("Load more")}
                        </Button>
                      )}
                    </article>
                  )}
                  <h2 className="font-medium">{ui("Outgoing replies")}</h2>
                  {outbox.data?.map((o) => (
                    <article key={o.id} className="space-y-2 border border-border p-4">
                      <div className="flex justify-between gap-3">
                        <strong>{o.body.subject}</strong>
                        <span>{ui(emailStateLabels[o.state] ?? "Needs review")}</span>
                      </div>
                      <p className="text-sm">{o.body.to.join(", ")}</p>
                      <ReplyPreview text={o.body.text} html={o.body.html} />
                      {o.state === "held" && (
                        <Button
                          disabled={busy}
                          onClick={() =>
                            void perform(async () => {
                              await api.post(`${root}/inboxes/${selected}/outbox/${o.id}/approve`);
                            })
                          }
                        >
                          {ui("Approve and send this reply")}
                        </Button>
                      )}
                      {o.state === "held" && (
                        <Button
                          variant="outline"
                          disabled={busy}
                          onClick={() =>
                            void perform(async () => {
                              await api.post(`${root}/inboxes/${selected}/outbox/${o.id}/reject`);
                            })
                          }
                        >
                          {ui("Reject reply")}
                        </Button>
                      )}
                    </article>
                  ))}
                </section>
              </div>
            )}
            {tab === "templates" && (
              <div className="grid gap-6 lg:grid-cols-2">
                <section className="space-y-4">
                  {templates.data?.map((t) => (
                    <div className="border border-border p-3" key={t.id}>
                      {t.name} · v{t.version}
                    </div>
                  ))}
                  <form
                    className="space-y-3"
                    onSubmit={(e) => {
                      e.preventDefault();
                      void perform(async () => {
                        await api.post(`${root}/templates`, template);
                      });
                    }}
                  >
                    <Label htmlFor="template-name">{ui("Template name")}</Label>
                    <Input
                      id="template-name"
                      required
                      value={templateName}
                      onChange={(e) => setTemplateName(e.target.value)}
                    />
                    <Label htmlFor="template-subject">{ui("Subject")}</Label>
                    <Input
                      id="template-subject"
                      value={subject}
                      onChange={(e) => setSubject(e.target.value)}
                    />
                    <Label htmlFor="template-html">HTML</Label>
                    <Textarea
                      id="template-html"
                      value={html}
                      onChange={(e) => setHtml(e.target.value)}
                    />
                    <Label htmlFor="template-text">{ui("Plain text")}</Label>
                    <Textarea
                      id="template-text"
                      value={text}
                      onChange={(e) => setText(e.target.value)}
                    />
                    <Label htmlFor="template-vars">{ui("Sample variables")}</Label>
                    <Textarea
                      id="template-vars"
                      value={variables}
                      onChange={(e) => setVariables(e.target.value)}
                    />
                    <div className="flex gap-2">
                      <Button type="submit" disabled={busy}>
                        {ui("Save template")}
                      </Button>
                      <Button
                        type="button"
                        variant="outline"
                        disabled={busy}
                        onClick={() =>
                          void perform(async () => {
                            setPreview(
                              (
                                await api.post<{ subject: string; html: string; text: string }>(
                                  `${root}/templates/preview`,
                                  { template, variables: JSON.parse(variables) },
                                )
                              ).data,
                            );
                          })
                        }
                      >
                        {ui("Preview")}
                      </Button>
                    </div>
                  </form>
                </section>
                {preview && (
                  <section className="space-y-3 border border-border p-4">
                    <h2>{preview.subject}</h2>
                    <Label>{ui("Plain text")}</Label>
                    <pre className="whitespace-pre-wrap break-words">{preview.text}</pre>
                    <Label>HTML</Label>
                    <BodyPreview html={preview.html} />
                  </section>
                )}
              </div>
            )}
            <OrganizationAccountDialog
              provider={provider}
              account={editingAccount}
              open={connecting}
              onOpenChange={setConnecting}
              onSaved={setNotice}
              description={ui("Connect an account for organization email inboxes.")}
            />
          </>
        )}
      </PageMain>
    </PageContainer>
  );
}

function ReplyPreview({ text, html }: { text: string; html?: string }) {
  const ui = useUiTranslations();
  const [view, setView] = useState<"text" | "html">("text");
  return (
    <div className="space-y-3">
      <div className="flex gap-2">
        <Button variant={view === "text" ? "default" : "outline"} onClick={() => setView("text")}>
          {ui("Text")}
        </Button>
        {html && (
          <Button variant={view === "html" ? "default" : "outline"} onClick={() => setView("html")}>
            {ui("HTML")}
          </Button>
        )}
      </div>
      {view === "html" && html ? (
        <BodyPreview html={html} />
      ) : (
        <pre className="whitespace-pre-wrap break-words text-sm">{text}</pre>
      )}
    </div>
  );
}

function BodyPreview({ html }: { html: string }) {
  const ui = useUiTranslations();
  return (
    <iframe
      title={ui("Email preview")}
      sandbox=""
      referrerPolicy="no-referrer"
      className="min-h-64 w-full border border-border bg-background"
      srcDoc={
        '<meta http-equiv="Content-Security-Policy" content="default-src &apos;none&apos;; style-src &apos;unsafe-inline&apos;; img-src data:">' +
        html
      }
    />
  );
}
