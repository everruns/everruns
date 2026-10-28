"use client";
import { useCallback, useEffect, useRef, useState } from "react";
import { MessageContent } from "./message-content";
import { FilePreview, downloadFile } from "./file-preview";
import { WorkLog, workForMessages } from "./work-log";
import { conversationMessages } from "./transcript";
import { SharedChat } from "./shared-chat";

type Config = {
  name: string;
  description?: string;
  interface_mode: "single_with_threads" | "multi_chat";
  features: { threads: boolean; files: boolean };
  models?: Array<{ id: string; name: string }>;
};
type Question = {
  id: string;
  question: string;
  options?: Array<{ label: string; description?: string }>;
  multi_select?: boolean;
  kind?: string;
  allow_other?: boolean;
};
type Session = {
  id: string;
  title?: string;
  parent_session_id?: string;
  status: string;
  archived?: boolean;
  pinned?: boolean;
  pending_approvals?: Array<{
    tool_call_id: string;
    answerable: boolean;
    tool_name?: string;
    arguments?: unknown;
  }>;
  pending_questions?: Array<{ tool_call_id: string; questions: Question[] }>;
};
type Event = { id?: string; sequence?: number; type: string; data: Record<string, unknown> };
type FileInfo = { name: string; path: string; is_directory: boolean; is_readonly?: boolean };
type FileContent = FileInfo & { content?: string; encoding?: string; revision?: string };
type Work = {
  id: string;
  title?: string;
  kind: string;
  state: string;
  summary?: string;
  links?: { child_session_id?: string };
};
async function api<T>(path: string, method = "GET", body?: unknown): Promise<T> {
  const response = await fetch(`/api/${path}`, {
    method,
    headers:
      body === undefined
        ? {}
        : { "Content-Type": "application/json", "Idempotency-Key": crypto.randomUUID() },
    body: body === undefined ? undefined : JSON.stringify(body),
    cache: "no-store",
  });
  if (response.status === 401) throw new Error("Sign in to continue");
  if (!response.ok) {
    const data = await response.json().catch(() => null);
    const detail =
      typeof data?.detail === "string"
        ? data.detail
        : typeof data?.error === "string"
          ? data.error
          : "The request could not be completed";
    throw new Error(detail);
  }
  return response.json();
}
export function Chat({ config }: { config: Config }) {
  const [share, setShare] = useState<string | null | undefined>(undefined);
  useEffect(() => {
    setShare(new URLSearchParams(window.location.search).get("share"));
  }, []);
  if (share === undefined) return <p role="status">Loading chat…</p>;
  return share ? <SharedChat token={share} name={config.name} /> : <WorkChat config={config} />;
}
function WorkChat({ config }: { config: Config }) {
  const [sessions, setSessions] = useState<Session[]>([]),
    [selected, setSelected] = useState<string | null>(null),
    [main, setMain] = useState<string | null>(null);
  const [session, setSession] = useState<Session | null>(null),
    [events, setEvents] = useState<Event[]>([]),
    [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false),
    [error, setError] = useState<string | null>(null),
    [signedIn, setSignedIn] = useState(true);
  const [work, setWork] = useState<Work[]>([]),
    [panel, setPanel] = useState<"work" | "files" | null>(null),
    [files, setFiles] = useState<FileInfo[]>([]),
    [path, setPath] = useState("/workspace"),
    [file, setFile] = useState<FileContent | null>(null),
    [edit, setEdit] = useState("");
  const [upload, setUpload] = useState<File[]>([]),
    [sidebar, setSidebar] = useState(false);
  const [search, setSearch] = useState(""),
    [archived, setArchived] = useState(false),
    [nextPage, setNextPage] = useState<string | null>(null);
  const [notice, setNotice] = useState(""),
    [model, setModel] = useState(""),
    [effort, setEffort] = useState("");
  const [preview, setPreview] = useState(true);
  const sending = useRef(false);
  const end = useRef<HTMLDivElement>(null),
    generation = useRef(0),
    lastSequence = useRef(0);
  const fail = useCallback((error: unknown) => {
    const text = error instanceof Error ? error.message : "Chat temporarily unavailable";
    setError(text);
    if (text === "Sign in to continue") setSignedIn(false);
  }, []);
  useEffect(() => {
    const timer = setInterval(async () => {
      try {
        const response = await fetch("/api/chat/config", { cache: "no-store" });
        if (!response.ok) return;
        const next: Config = await response.json();
        if (
          next.interface_mode !== config.interface_mode ||
          next.name !== config.name ||
          next.description !== config.description ||
          next.features.files !== config.features.files ||
          next.features.threads !== config.features.threads ||
          JSON.stringify(next.models) !== JSON.stringify(config.models)
        )
          window.location.reload();
      } catch {
        /* Normal API status polling reports connection errors. */
      }
    }, 10000);
    return () => clearInterval(timer);
  }, [config]);
  useEffect(() => {
    const restore = () => {
      const value = new URLSearchParams(window.location.search).get("session");
      setSelected(
        value && value.length <= 128 && /^session_[A-Za-z0-9_-]+$/.test(value) ? value : null,
      );
    };
    restore();
    window.addEventListener("popstate", restore);
    return () => window.removeEventListener("popstate", restore);
  }, []);
  useEffect(() => {
    if (!selected) return;
    const url = new URL(window.location.href);
    if (url.searchParams.get("session") === selected) return;
    url.searchParams.set("session", selected);
    window.history.replaceState(null, "", url);
  }, [selected]);
  const refresh = useCallback(async () => {
    const list = await api<{ data: Session[]; next_page_token?: string }>(
      "sessions?limit=100&include_archived=true",
    );
    setNextPage(list.next_page_token ?? null);
    const roots = list.data.filter((session) => !session.parent_session_id);
    setSessions(roots);
    setSignedIn(true);
    if (config.interface_mode === "single_with_threads") {
      const root = await api<Session>("chat/main", "POST", {});
      setMain(root.id);
      setSelected((current) => current ?? root.id);
    } else setSelected((current) => current ?? roots[0]?.id ?? null);
  }, [config.interface_mode]);
  useEffect(() => {
    refresh().catch(fail);
  }, [refresh, fail]);
  useEffect(() => {
    if (!selected) return;
    const epoch = ++generation.current;
    let stopped = false;
    let controller: AbortController | undefined;
    setEvents([]);
    setSession(null);
    setFile(null);
    setFiles([]);
    setWork([]);
    lastSequence.current = 0;
    const append = (incoming: Event[]) => {
      if (stopped || generation.current !== epoch) return;
      setEvents((old) => {
        const byId = new Map(
          old.map((event) => [event.id ?? `${event.type}:${event.sequence}`, event]),
        );
        for (const event of incoming) {
          byId.set(event.id ?? `${event.type}:${event.sequence}`, event);
          lastSequence.current = Math.max(lastSequence.current, event.sequence ?? 0);
        }
        return [...byId.values()];
      });
    };
    const status = async () => {
      const value = await api<Session>(`sessions/${selected}`);
      if (!stopped) {
        setSession(value);
        setSessions((old) =>
          old.map((item) => (item.id === value.id ? { ...item, ...value } : item)),
        );
        setSignedIn(true);
      }
      const result = await api<{ data: Work[] }>(
        `sessions/${config.interface_mode === "single_with_threads" && main ? main : selected}/work`,
      );
      if (!stopped) setWork(result.data);
    };
    const stream = async () => {
      while (!stopped) {
        try {
          controller = new AbortController();
          const response = await fetch(
            `/api/sessions/${selected}/sse?after_sequence=${lastSequence.current}`,
            { signal: controller.signal },
          );
          if (!response.ok) {
            if (response.status === 401) throw new Error("Sign in to continue");
            throw new Error("Reconnecting to your conversation…");
          }
          const reader = response.body!.getReader(),
            decoder = new TextDecoder();
          let buffer = "";
          while (!stopped) {
            const chunk = await reader.read();
            if (chunk.done) break;
            buffer += decoder.decode(chunk.value, { stream: true }).replace(/\r\n/g, "\n");
            let boundary;
            while ((boundary = buffer.indexOf("\n\n")) >= 0) {
              const frame = buffer.slice(0, boundary);
              buffer = buffer.slice(boundary + 2);
              const data = frame
                .split("\n")
                .filter((line) => line.startsWith("data:"))
                .map((line) => line.slice(5).trimStart())
                .join("\n");
              if (data) {
                try {
                  const event = JSON.parse(data);
                  if (event.type && event.data) append([event]);
                } catch {
                  /* heartbeat/control frame */
                }
              }
            }
          }
        } catch (error) {
          if (!stopped) fail(error);
        }
        if (!stopped) await new Promise((resolve) => setTimeout(resolve, 1500));
      }
    };
    status().catch(fail);
    stream();
    const interval = setInterval(() => status().catch(fail), 4000);
    return () => {
      stopped = true;
      controller?.abort();
      clearInterval(interval);
    };
  }, [selected, main, config.interface_mode, config.features.threads, fail]);
  useEffect(() => {
    end.current?.scrollIntoView({ behavior: "smooth" });
  }, [events.length]);
  const create = async () => {
    try {
      setError(null);
      const chat = await api<Session>("sessions", "POST", {});
      choose(chat.id);
      setSessions((old) => [chat, ...old]);
      setSidebar(false);
    } catch (error) {
      fail(error);
    }
  };
  const send = async (text = draft) => {
    if (sending.current || (!text.trim() && !upload.length) || session?.archived) return;
    sending.current = true;
    const epoch = generation.current;
    setBusy(true);
    setError(null);
    try {
      let id = selected;
      if (!id) {
        const chat = await api<Session>("sessions", "POST", {});
        id = chat.id;
        setSelected(id);
        setSessions((old) => [chat, ...old]);
      }
      const attachments = await Promise.all(
        upload.map(async (file) => ({
          name: file.name,
          media_type: file.type || "application/octet-stream",
          data: await new Promise<string>((resolve, reject) => {
            const reader = new FileReader();
            reader.onload = () => resolve(String(reader.result).split(",")[1]);
            reader.onerror = reject;
            reader.readAsDataURL(file);
          }),
        })),
      );
      await api(`sessions/${id}/messages`, "POST", {
        message: { role: "user", content: text.trim() ? [{ type: "text", text }] : [] },
        files: attachments,
        controls: {
          ...(model ? { model_id: model } : {}),
          ...(effort ? { reasoning: { effort } } : {}),
        },
      });
      if (epoch === generation.current || !selected) {
        setDraft("");
        setUpload([]);
      }
    } catch (error) {
      fail(error);
    } finally {
      sending.current = false;
      setBusy(false);
    }
  };
  const browse = async (next: string) => {
    if (!selected) return;
    const epoch = generation.current;
    try {
      const data = await api<{ data?: FileInfo[]; file?: FileContent; revision?: string }>(
        `sessions/${selected}/files?path=${encodeURIComponent(next)}`,
      );
      if (epoch !== generation.current) return;
      setError(null);
      if (data.file) {
        setFile({ ...data.file, revision: data.revision });
        setEdit(data.file.content ?? "");
        setPreview(/\.(html?|svg|csv|md|png|jpe?g|webp|gif)$/i.test(data.file.name));
      } else {
        setPath(next);
        setFiles(data.data ?? []);
        setFile(null);
      }
    } catch (error) {
      fail(error);
    }
  };
  const saveFile = async () => {
    if (!selected || !file) return;
    try {
      await api(`sessions/${selected}/files`, "POST", {
        path: file.path,
        expected_revision: file.revision,
        content: edit,
        encoding: file.encoding ?? "text",
      });
      await browse(file.path);
    } catch (error) {
      fail(error);
    }
  };
  const settings = async (change: Record<string, unknown>) => {
    if (!selected) return;
    try {
      const next = await api<Session>(`sessions/${selected}/settings`, "POST", change);
      setSession(next);
      await refresh();
    } catch (error) {
      fail(error);
    }
  };
  const addFiles = (incoming: FileList | File[]) => {
    if (busy) return;
    const files = [...upload, ...Array.from(incoming)];
    if (files.length > 3 || files.reduce((size, file) => size + file.size, 0) > 4 * 1024 * 1024) {
      fail(new Error("Attach up to 3 files, totaling at most 4 MB"));
      return;
    }
    setUpload(files);
  };
  const messages = conversationMessages(events);
  const turnWork = workForMessages(events);
  const turnState = events.findLast((event) =>
    ["turn.started", "turn.completed", "turn.failed", "turn.cancelled"].includes(event.type),
  );
  const running = turnState
    ? turnState.type === "turn.started"
    : ["active", "started"].includes(session?.status ?? "");
  const choose = (id: string) => {
    const url = new URL(window.location.href);
    url.searchParams.set("session", id);
    window.history.pushState(null, "", url);
    setSelected(id);
    setDraft("");
    setUpload([]);
    setNotice("");
    setSidebar(false);
    setWork([]);
  };
  return (
    <div
      className="app"
      onDragOver={(event) => event.preventDefault()}
      onDrop={(event) => {
        event.preventDefault();
        addFiles(event.dataTransfer.files);
      }}
    >
      <aside className={sidebar ? "sidebar open" : "sidebar"}>
        <div className="brand">
          ◉ <span>{config.name}</span>
        </div>
        {config.interface_mode === "multi_chat" ? (
          <button className="primary" onClick={create}>
            ＋ New conversation
          </button>
        ) : (
          <button className="primary" onClick={() => main && choose(main)}>
            Main conversation
          </button>
        )}
        <div className="section-label">
          {config.interface_mode === "multi_chat" ? "Conversations" : "Threads"}
        </div>
        <input
          aria-label="Search conversations"
          className="conversation-search"
          placeholder="Search conversations…"
          value={search}
          onChange={(event) => setSearch(event.target.value)}
        />
        {config.interface_mode === "multi_chat" && (
          <button className="archive-filter" onClick={() => setArchived(!archived)}>
            {archived ? "Show active" : "Show archived"}
          </button>
        )}
        <nav aria-label="Conversations">
          {config.interface_mode === "multi_chat"
            ? sessions
                .filter(
                  (chat) =>
                    Boolean(chat.archived) === archived &&
                    (chat.title ?? "New conversation").toLowerCase().includes(search.toLowerCase()),
                )
                .sort((a, b) => Number(Boolean(b.pinned)) - Number(Boolean(a.pinned)))
                .map((chat) => (
                  <button
                    key={chat.id}
                    className={selected === chat.id ? "row selected" : "row"}
                    onClick={() => choose(chat.id)}
                  >
                    {chat.pinned ? "⌖ " : ""}
                    {chat.title || "New conversation"}
                  </button>
                ))
            : work
                .filter(
                  (task) =>
                    task.links?.child_session_id &&
                    (task.title ?? "Work thread").toLowerCase().includes(search.toLowerCase()),
                )
                .map((task) => (
                  <button
                    key={task.id}
                    className="row"
                    onClick={() => choose(task.links!.child_session_id!)}
                  >
                    {task.title || "Work thread"}
                    <small>{task.state}</small>
                  </button>
                ))}
          {nextPage && config.interface_mode === "multi_chat" && (
            <button
              onClick={async () => {
                try {
                  const page = await api<{ data: Session[]; next_page_token?: string }>(
                    `sessions?include_archived=true&limit=100&page_token=${encodeURIComponent(nextPage)}`,
                  );
                  setSessions((old) => [
                    ...old,
                    ...page.data.filter(
                      (item) =>
                        !item.parent_session_id && !old.some((existing) => existing.id === item.id),
                    ),
                  ]);
                  setNextPage(page.next_page_token ?? null);
                } catch (error) {
                  fail(error);
                }
              }}
            >
              More conversations
            </button>
          )}
        </nav>
        <footer>
          <span>Work Chat</span>
          {signedIn ? (
            <button
              onClick={async () => {
                await fetch("/auth/logout", { method: "POST" });
                location.reload();
              }}
            >
              Sign out
            </button>
          ) : (
            <a
              href={
                selected ? `/auth/start?session=${encodeURIComponent(selected)}` : "/auth/start"
              }
            >
              Sign in
            </a>
          )}
        </footer>
      </aside>
      <main>
        <header>
          <button
            className="mobile-menu"
            aria-label="Toggle conversations"
            onClick={() => setSidebar(!sidebar)}
          >
            ☰
          </button>
          <div>
            <strong>{session?.title || config.name}</strong>
            <small>{running ? "Working" : "Ready"}</small>
          </div>
          <div className="header-actions">
            {selected && (
              <details className="conversation-menu">
                <summary aria-label="Conversation actions">•••</summary>
                <div>
                  {(config.interface_mode === "multi_chat" || session?.parent_session_id) && (
                    <>
                      <button
                        onClick={() => {
                          const title = prompt("Conversation title", session?.title ?? "");
                          if (title !== null) void settings({ title });
                        }}
                      >
                        Rename
                      </button>
                      <button onClick={() => settings({ pinned: !session?.pinned })}>
                        {session?.pinned ? "Unpin" : "Pin"}
                      </button>
                      <button onClick={() => settings({ archived: !session?.archived })}>
                        {session?.archived ? "Reopen" : "Archive"}
                      </button>
                    </>
                  )}
                  <button
                    onClick={async () => {
                      if (
                        !confirm(
                          "Share this conversation's messages and widgets with anyone who has the link? Workspace files stay private.",
                        )
                      )
                        return;
                      try {
                        const result = await api<{ token: string }>(
                          `sessions/${selected}/share`,
                          "POST",
                          {},
                        );
                        const url = new URL(location.href);
                        url.search = "";
                        url.searchParams.set("share", result.token);
                        await navigator.clipboard.writeText(url.toString());
                        setNotice("Read-only link copied. A new link replaces the previous one.");
                      } catch (error) {
                        fail(error);
                      }
                    }}
                  >
                    Share read-only link
                  </button>
                  <button
                    onClick={async () => {
                      try {
                        await api(`sessions/${selected}/share`, "DELETE");
                        setNotice("Public links revoked.");
                      } catch (error) {
                        fail(error);
                      }
                    }}
                  >
                    Revoke public links
                  </button>
                </div>
              </details>
            )}

            <button onClick={() => setPanel(panel === "work" ? null : "work")}>
              {config.features.threads ? "Threads" : "Work"} <span>{work.length || ""}</span>
            </button>
            {config.features.files && (
              <button
                onClick={() => {
                  setPanel(panel === "files" ? null : "files");
                  browse(path);
                }}
              >
                Workspace
              </button>
            )}
          </div>
        </header>
        <section className="conversation" aria-label="Conversation">
          {!messages.length && (
            <div className="welcome">
              <div className="mark">◉</div>
              <h1>{config.name}</h1>
              <p>
                {config.description || "Bring a question, a task, or something you want to build."}
              </p>
            </div>
          )}
          {messages.map((message) => (
            <article
              key={message.id}
              className={message.role === "user" ? "message user" : "message assistant"}
            >
              <span className="sr-only">{message.role}</span>
              {message.role !== "user" && <WorkLog events={turnWork.logs.get(message.id) ?? []} />}
              <MessageContent
                text={message.text}
                streaming={running && message.id === messages.at(-1)?.id}
                onMessage={(text) => void send(text)}
                onFile={
                  config.features.files
                    ? (path) => {
                        setPanel("files");
                        void browse(path);
                      }
                    : undefined
                }
              />
              {message.role !== "user" && (
                <button
                  className="copy-message"
                  aria-label="Copy message"
                  onClick={() =>
                    navigator.clipboard
                      .writeText(message.text)
                      .then(() => setNotice("Message copied."))
                      .catch(fail)
                  }
                >
                  Copy
                </button>
              )}
            </article>
          ))}
          {running && <WorkLog events={turnWork.pending} running />}
          {!!work.length && (
            <div className="work-chips">
              {work.slice(0, 8).map((task) => (
                <button
                  key={task.id}
                  onClick={() =>
                    task.links?.child_session_id
                      ? choose(task.links.child_session_id)
                      : setPanel("work")
                  }
                >
                  {task.title || "Background work"} <small>{task.state.replaceAll("_", " ")}</small>
                </button>
              ))}
            </div>
          )}
          {session?.pending_approvals?.map((approval) => (
            <section className="request" key={approval.tool_call_id}>
              <strong>
                {approval.answerable
                  ? `Approve ${approval.tool_name}?`
                  : "Waiting for operator approval"}
              </strong>
              {approval.answerable && (
                <>
                  <pre>{JSON.stringify(approval.arguments, null, 2)}</pre>
                  <button
                    onClick={() =>
                      api(`sessions/${selected}/tool-approvals`, "POST", {
                        decisions: [{ tool_call_id: approval.tool_call_id, decision: "allow" }],
                      }).catch(fail)
                    }
                  >
                    Allow once
                  </button>
                  <button
                    onClick={() =>
                      api(`sessions/${selected}/tool-approvals`, "POST", {
                        decisions: [{ tool_call_id: approval.tool_call_id, decision: "reject" }],
                      }).catch(fail)
                    }
                  >
                    Reject
                  </button>
                </>
              )}
            </section>
          ))}
          {session?.pending_questions?.map((request) => (
            <QuestionForm
              key={request.tool_call_id}
              questions={request.questions}
              onSubmit={(answers) =>
                api(`sessions/${selected}/question-answers`, "POST", {
                  tool_call_id: request.tool_call_id,
                  status: "answered",
                  answers,
                })
                  .then(() => {})
                  .catch(fail)
              }
            />
          ))}
          <div ref={end} />
        </section>
        <div className="composer-area">
          {notice && (
            <p role="status" className="notice">
              {notice}
            </p>
          )}
          {session?.archived && (
            <p className="notice">
              Archived conversation.{" "}
              <button onClick={() => settings({ archived: false })}>Reopen</button>
            </p>
          )}
          {error && (
            <div role="alert" className="error">
              {error}
              {!signedIn && (
                <>
                  {" "}
                  ·{" "}
                  <a
                    href={
                      selected
                        ? `/auth/start?session=${encodeURIComponent(selected)}`
                        : "/auth/start"
                    }
                  >
                    Sign in
                  </a>
                </>
              )}
            </div>
          )}
          {!signedIn ? (
            <div className="signin">
              <p>Sign in to start working with {config.name}.</p>
              <a
                className="primary"
                href={
                  selected ? `/auth/start?session=${encodeURIComponent(selected)}` : "/auth/start"
                }
              >
                Continue to sign in →
              </a>
            </div>
          ) : (
            <form
              className="composer"
              onSubmit={(event) => {
                event.preventDefault();
                send();
              }}
            >
              {upload.map((file, index) => (
                <div className="attachment" key={`${file.name}-${index}`}>
                  {file.name}
                  <button
                    type="button"
                    aria-label={`Remove ${file.name}`}
                    onClick={() => setUpload((old) => old.filter((_, i) => i !== index))}
                  >
                    ×
                  </button>
                </div>
              ))}
              <textarea
                aria-label="Message"
                disabled={session?.archived || busy}
                onPaste={(event) => {
                  if (event.clipboardData.files.length) {
                    event.preventDefault();
                    addFiles(event.clipboardData.files);
                  }
                }}
                placeholder="Ask anything, or describe the work…"
                value={draft}
                onChange={(event) => setDraft(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
                    event.preventDefault();
                    send();
                  }
                }}
              />
              <div className="composer-controls">
                <label className="upload">
                  ＋ Attach
                  <input
                    type="file"
                    disabled={busy}
                    multiple
                    aria-label="Attach files"
                    onChange={(event) => {
                      if (event.target.files) addFiles(event.target.files);
                      event.target.value = "";
                    }}
                  />
                </label>
                {!!config.models?.length && (
                  <select
                    aria-label="Model"
                    value={model}
                    onChange={(event) => setModel(event.target.value)}
                  >
                    <option value="">Agent default</option>
                    {config.models.map((model) => (
                      <option key={model.id} value={model.id}>
                        {model.name}
                      </option>
                    ))}
                  </select>
                )}
                <select
                  aria-label="Reasoning effort"
                  value={effort}
                  onChange={(event) => setEffort(event.target.value)}
                >
                  <option value="">Default effort</option>
                  <option value="low">Low</option>
                  <option value="medium">Medium</option>
                  <option value="high">High</option>
                </select>
                <span>Enter to send · Shift + Enter for a new line</span>
                {(running ||
                  !!session?.pending_questions?.length ||
                  !!session?.pending_approvals?.length) && (
                  <button
                    type="button"
                    onClick={() => api(`sessions/${selected}/cancel`, "POST", {}).catch(fail)}
                  >
                    Stop
                  </button>
                )}
                <button
                  className="primary"
                  disabled={busy || session?.archived || (!draft.trim() && !upload.length)}
                >
                  {busy ? "Sending…" : "Send ↑"}
                </button>
              </div>
            </form>
          )}
        </div>
      </main>
      {panel && (
        <aside className="context">
          <div className="context-header">
            <strong>
              {panel === "files"
                ? "Workspace"
                : config.features.threads
                  ? "Work & threads"
                  : "Work activity"}
            </strong>
            <button aria-label="Close panel" onClick={() => setPanel(null)}>
              ×
            </button>
          </div>
          {panel === "work" ? (
            work.map((task) => (
              <section className="task" key={task.id}>
                <small>{task.state}</small>
                <h3>{task.title || "Background work"}</h3>
                <p>{task.summary}</p>
                {task.links?.child_session_id && (
                  <button onClick={() => choose(task.links!.child_session_id!)}>
                    Open thread →
                  </button>
                )}
              </section>
            ))
          ) : (
            <>
              <div className="file-nav">
                <button onClick={() => browse("/workspace")}>Workspace</button>
                <button onClick={() => browse("/attachments")}>Attachments</button>
              </div>
              <p className="file-path">{path}</p>
              {file ? (
                <div className="file-preview">
                  <button onClick={() => setFile(null)}>← Files</button>
                  <h3>{file.name}</h3>
                  <button onClick={() => browse(file.path)}>Reload file</button>
                  <button onClick={() => downloadFile(file)}>Download</button>
                  {file.encoding !== "base64" && (
                    <button onClick={() => setPreview(!preview)}>
                      {preview ? "Edit source" : "Preview"}
                    </button>
                  )}
                  {preview ? (
                    <FilePreview file={file} />
                  ) : (
                    <>
                      <textarea
                        aria-label="File content"
                        value={edit}
                        readOnly={file.is_readonly}
                        onChange={(event) => setEdit(event.target.value)}
                      />
                      {!file.is_readonly && <button onClick={saveFile}>Save changes</button>}
                    </>
                  )}
                  {!file.is_readonly && (
                    <button
                      onClick={async () => {
                        if (!confirm(`Delete ${file.name}?`)) return;
                        try {
                          await api(
                            `sessions/${selected}/files?path=${encodeURIComponent(file.path)}`,
                            "DELETE",
                          );
                          browse(path);
                        } catch (error) {
                          fail(error);
                        }
                      }}
                    >
                      Delete file
                    </button>
                  )}
                </div>
              ) : (
                files.map((file) => (
                  <button className="row" key={file.path} onClick={() => browse(file.path)}>
                    {file.is_directory ? "▸" : "▤"} {file.name}
                  </button>
                ))
              )}
            </>
          )}
        </aside>
      )}
    </div>
  );
}
function QuestionForm({
  questions,
  onSubmit,
}: {
  questions: Question[];
  onSubmit: (
    answers: Array<{ id: string; selected: string[]; other_text: string | null }>,
  ) => Promise<void>;
}) {
  const [answers, setAnswers] = useState<
    Record<string, { selected: string[]; other_text: string }>
  >({});
  const [sending, setSending] = useState(false);
  return (
    <form
      className="request"
      onSubmit={async (event) => {
        event.preventDefault();
        setSending(true);
        try {
          await onSubmit(
            questions.map((q) => ({
              id: q.id,
              selected: answers[q.id]?.selected ?? [],
              other_text: answers[q.id]?.other_text || null,
            })),
          );
        } finally {
          setSending(false);
        }
      }}
    >
      {questions.map((question) => {
        const answer = answers[question.id] ?? { selected: [], other_text: "" };
        return (
          <fieldset key={question.id}>
            <legend>{question.question}</legend>
            {question.options?.map((option) => (
              <label className="question-option" key={option.label}>
                <input
                  type={question.multi_select ? "checkbox" : "radio"}
                  name={question.id}
                  checked={answer.selected.includes(option.label)}
                  onChange={() =>
                    setAnswers((old) => ({
                      ...old,
                      [question.id]: {
                        ...answer,
                        selected: question.multi_select
                          ? answer.selected.includes(option.label)
                            ? answer.selected.filter((v) => v !== option.label)
                            : [...answer.selected, option.label]
                          : [option.label],
                      },
                    }))
                  }
                />
                <span>
                  {option.label}
                  {option.description && <small>{option.description}</small>}
                </span>
              </label>
            ))}
            {(!question.options?.length || question.allow_other) && (
              <input
                aria-label={`Answer: ${question.question}`}
                placeholder={question.options?.length ? "Another answer…" : "Your answer…"}
                value={answer.other_text}
                onChange={(event) =>
                  setAnswers((old) => ({
                    ...old,
                    [question.id]: { ...answer, other_text: event.target.value },
                  }))
                }
              />
            )}
          </fieldset>
        );
      })}
      <button disabled={sending}>Continue</button>
    </form>
  );
}
