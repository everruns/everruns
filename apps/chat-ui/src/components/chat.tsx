"use client";
import { useCallback, useEffect, useRef, useState } from "react";
import { MessageContent } from "./message-content";
import { FilePreview, downloadFile } from "./file-preview";
import { WorkLog, workForMessages } from "./work-log";
import { conversationMessages } from "./transcript";
import { SharedChat } from "./shared-chat";
import { consumeSse } from "../client/sse";
import {
  recoveryKey,
  readRecovery,
  saveRecovery,
  saveDraft,
  clearRecovery,
  retryable,
  type PendingMessage,
} from "../client/recovery";

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
type Event = {
  id?: string;
  sequence?: number;
  type: string;
  data: Record<string, unknown>;
};
type FileInfo = {
  name: string;
  path: string;
  is_directory: boolean;
  is_readonly?: boolean;
};
type FileContent = FileInfo & {
  content?: string;
  encoding?: string;
  revision?: string;
};
type Work = {
  id: string;
  title?: string;
  kind: string;
  state: string;
  summary?: string;
  links?: { child_session_id?: string };
};
async function api<T>(
  path: string,
  method = "GET",
  body?: unknown,
  idempotencyKey?: string,
): Promise<T> {
  const response = await fetch(`/api/${path}`, {
    method,
    headers:
      body === undefined
        ? {}
        : {
            "Content-Type": "application/json",
            "Idempotency-Key": idempotencyKey ?? crypto.randomUUID(),
          },
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
  const [recoveryScope, setRecoveryScope] = useState<string | null | undefined>(undefined);
  const [pending, setPending] = useState<PendingMessage | undefined>();
  const [storageError, setStorageError] = useState(false);
  const recoveryRecords = useRef(new Map<string, { draft: string; pending?: PendingMessage }>());
  const busyRef = useRef(false);
  const accessDenied = useRef(false);
  const retired = useRef(false);
  const newConversationKey = useRef<string | undefined>(undefined);
  const currentRef = useRef({ selected, scope: recoveryScope, draft });
  currentRef.current = { selected, scope: recoveryScope, draft };
  const localKey = recoveryScope ? recoveryKey(recoveryScope, selected ?? "new") : undefined;
  const selectionKey = localKey ?? `memory:${selected ?? "new"}`;
  const store = (key: string, value: { draft: string; pending?: PendingMessage }) => {
    if (retired.current) return;
    const previous = recoveryRecords.current.get(key);
    recoveryRecords.current.set(key, value);
    if (!key.startsWith("memory:")) {
      try {
        if (!storageError && previous?.pending === value.pending) saveDraft(key, value.draft);
        else saveRecovery(key, value);
        setStorageError(false);
      } catch {
        setStorageError(true);
      }
    }
  };
  useEffect(() => {
    api<{ scope?: string | null }>("chat/recovery")
      .then((value) => {
        setRecoveryScope(value.scope ?? null);
        setStorageError(!value.scope);
      })
      .catch(failRecovery);
    function failRecovery(error: unknown) {
      setRecoveryScope(null);
      setStorageError(true);
      fail(error);
    }
  }, []);
  useEffect(() => {
    let value = recoveryRecords.current.get(selectionKey) ?? {
      draft: "",
      pending: undefined,
    };
    if (localKey) {
      try {
        value = readRecovery(localKey);
        setStorageError(false);
      } catch {
        setStorageError(true);
      }
    }
    recoveryRecords.current.set(selectionKey, value);
    setDraft(value.draft);
    setPending(value.pending);
    setUpload([]);
  }, [selectionKey, localKey]);
  const changeDraft = (text: string) => {
    setDraft(text);
    store(selectionKey, {
      ...recoveryRecords.current.get(selectionKey),
      draft: text,
    });
  };
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
  const end = useRef<HTMLDivElement>(null),
    generation = useRef(0),
    lastSequence = useRef(0);
  const fail = useCallback((error: unknown) => {
    const text = error instanceof Error ? error.message : "Chat temporarily unavailable";
    setError(text);
    if (text === "Sign in to continue") {
      accessDenied.current = true;
      setSignedIn(false);
    }
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
    if (!accessDenied.current) setSignedIn(true);
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
    let retryDelay = 1500;
    let wakeRetry: (() => void) | undefined;
    let retryTimer: ReturnType<typeof setTimeout> | undefined;
    setEvents([]);
    setSession(null);
    setFile(null);
    setFiles([]);
    setWork([]);
    lastSequence.current = 0;
    const append = (incoming: Event[]) => {
      if (stopped || generation.current !== epoch) return;
      for (const event of incoming)
        lastSequence.current = Math.max(lastSequence.current, event.sequence ?? 0);
      setEvents((old) => {
        const byId = new Map(
          old.map((event) => [event.id ?? `${event.type}:${event.sequence}`, event]),
        );
        for (const event of incoming) {
          byId.set(event.id ?? `${event.type}:${event.sequence}`, event);
        }
        return [...byId.values()].slice(-5000);
      });
    };
    const report = (error: unknown) => {
      if (!stopped && generation.current === epoch) fail(error);
    };
    const status = async () => {
      if (accessDenied.current) return;
      const value = await api<Session>(`sessions/${selected}`);
      if (!stopped && !accessDenied.current) {
        setSession(value);
        setSessions((old) =>
          old.map((item) => (item.id === value.id ? { ...item, ...value } : item)),
        );
        setSignedIn(true);
      }
      const result = await api<{ data: Work[] }>(
        `sessions/${config.interface_mode === "single_with_threads" && main ? main : selected}/work`,
      );
      if (!stopped && !accessDenied.current) setWork(result.data);
    };
    const stream = async () => {
      while (!stopped) {
        try {
          controller = new AbortController();
          const response = await fetch(
            `/api/sessions/${selected}/sse?after_sequence=${lastSequence.current}`,
            { signal: controller.signal },
          );
          if (stopped || generation.current !== epoch) {
            await response.body?.cancel();
            return;
          }
          if (!response.ok) {
            if ([401, 403, 404].includes(response.status)) {
              report(
                new Error(
                  response.status === 401
                    ? "Sign in to continue"
                    : "This conversation is no longer available",
                ),
              );
              return;
            }
            throw new Error("Reconnecting to your conversation…");
          }
          retryDelay = 1500;
          setError(null);
          await consumeSse(response.body!, (_type, data) => {
            try {
              const event = JSON.parse(data);
              if (event.type && event.data) append([event]);
            } catch {
              /* heartbeat/control frame */
            }
          });
        } catch (error) {
          report(error);
        }
        if (!stopped)
          await new Promise<void>((resolve) => {
            const done = () => {
              clearTimeout(retryTimer);
              wakeRetry = undefined;
              resolve();
            };
            wakeRetry = done;
            retryTimer = setTimeout(done, retryDelay * (0.8 + Math.random() * 0.4));
            retryDelay = Math.min(retryDelay * 2, 30000);
          });
      }
    };
    const wake = () => {
      if (navigator.onLine) {
        wakeRetry?.();
        void status().catch(report);
      }
    };
    window.addEventListener("online", wake);
    window.addEventListener("focus", wake);
    status().catch(report);
    void stream();
    const interval = setInterval(() => status().catch(report), 4000);
    return () => {
      stopped = true;
      controller?.abort();
      wakeRetry?.();
      clearTimeout(retryTimer);
      window.removeEventListener("online", wake);
      window.removeEventListener("focus", wake);
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
  const send = async (text = draft, { fromWidget = false, retry = false } = {}) => {
    const submittedFiles = fromWidget ? [] : upload;
    if (
      retired.current ||
      busyRef.current ||
      session?.archived ||
      recoveryScope === undefined ||
      (pending && !retry) ||
      (!pending && !text.trim() && !submittedFiles.length)
    )
      return;
    const submittedDraft = draft,
      scope = recoveryScope;
    busyRef.current = true;
    setBusy(true);
    setError(null);
    let id = selected;
    const currentIntent = () => !retired.current && currentRef.current.scope === scope;
    try {
      if (!id) {
        const chat = await api<Session>(
          "sessions",
          "POST",
          {},
          (newConversationKey.current ??= crypto.randomUUID()),
        );
        if (!currentIntent()) return;
        newConversationKey.current = undefined;
        id = chat.id;
        setSelected(id);
        setSessions((old) => [chat, ...old]);
      }
      if (!currentIntent()) return;
      const key = scope ? recoveryKey(scope, id) : `memory:${id}`;
      let submission = pending;
      if (!submission) {
        const attachments = await Promise.all(
          submittedFiles.map(async (file) => ({
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
        if (!currentIntent()) return;
        submission = {
          key: crypto.randomUUID(),
          createdAt: Date.now(),
          fromWidget,
          body: {
            message: {
              role: "user",
              content: text.trim() ? [{ type: "text", text }] : [],
            },
            files: attachments,
            controls: {
              ...(model ? { model_id: model } : {}),
              ...(effort ? { reasoning: { effort } } : {}),
            },
          },
        };
        store(key, { draft: submittedDraft, pending: submission });
        if (!selected && scope) store(recoveryKey(scope, "new"), { draft: "" });
        if (
          currentRef.current.scope === scope &&
          (currentRef.current.selected === id || currentRef.current.selected === null)
        )
          setPending(submission);
      }
      if (!retryable(submission))
        throw new Error(
          "This pending send is older than the retry window. Review the conversation before sending again.",
        );
      if (!currentIntent()) return;
      await api(`sessions/${id}/messages`, "POST", submission.body, submission.key);
      const live = currentRef.current.scope === scope && currentRef.current.selected === id;
      const originalText = (
        (
          submission.body as {
            message?: { content?: Array<{ type: string; text?: string }> };
          }
        ).message?.content ?? []
      )
        .filter((part) => part.type === "text")
        .map((part) => part.text ?? "")
        .join("\n");
      const savedDraft = recoveryRecords.current.get(key)?.draft ?? submittedDraft;
      const nextDraft = !submission.fromWidget && savedDraft === originalText ? "" : savedDraft;
      store(key, { draft: nextDraft });
      if (live) {
        setDraft(nextDraft);
        setPending(undefined);
        if (!submission.fromWidget) setUpload([]);
      }
    } catch (error) {
      if (
        currentRef.current.scope === scope &&
        (currentRef.current.selected === id || currentRef.current.selected === null)
      )
        fail(error);
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  };
  const discard = () => {
    store(selectionKey, { draft });
    setPending(undefined);
  };
  const browse = async (next: string) => {
    if (!selected) return;
    const epoch = generation.current;
    try {
      const data = await api<{
        data?: FileInfo[];
        file?: FileContent;
        revision?: string;
      }>(`sessions/${selected}/files?path=${encodeURIComponent(next)}`);
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
                  const page = await api<{
                    data: Session[];
                    next_page_token?: string;
                  }>(
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
                retired.current = true;
                accessDenied.current = true;
                setRecoveryScope(null);
                setSignedIn(false);
                clearRecovery();
                recoveryRecords.current.clear();
                setDraft("");
                setPending(undefined);
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
                          "Share this conversation's messages and widgets, including future messages, with anyone who has the link? Workspace files stay private.",
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
                        try {
                          await navigator.clipboard.writeText(url.toString());
                          setNotice("Read-only link copied. A new link replaces the previous one.");
                        } catch {
                          setNotice(`Read-only link: ${url.toString()}`);
                        }
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
                onMessage={
                  !session?.archived && !running && !busy
                    ? (text) => send(text, { fromWidget: true })
                    : undefined
                }
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
                        decisions: [
                          {
                            tool_call_id: approval.tool_call_id,
                            decision: "allow",
                          },
                        ],
                      }).catch(fail)
                    }
                  >
                    Allow once
                  </button>
                  <button
                    onClick={() =>
                      api(`sessions/${selected}/tool-approvals`, "POST", {
                        decisions: [
                          {
                            tool_call_id: approval.tool_call_id,
                            decision: "reject",
                          },
                        ],
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
          {storageError && (
            <p role="alert">
              Recovery storage is unavailable. Keep this tab open and copy unsent text before
              leaving.
            </p>
          )}
          {pending && (
            <div className="request" role="status">
              Message delivery is unconfirmed. Retry checks the original send. Discard only removes
              the local copy.
              <button
                type="button"
                disabled={busy}
                onClick={() => void send(draft, { retry: true })}
              >
                Retry message
              </button>
              <button type="button" disabled={busy} onClick={discard}>
                Discard local copy
              </button>
            </div>
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
                onPaste={(event) => {
                  if (event.clipboardData.files.length) {
                    event.preventDefault();
                    addFiles(event.clipboardData.files);
                  }
                }}
                placeholder="Ask anything, or describe the work…"
                value={draft}
                disabled={session?.archived || recoveryScope === undefined}
                onChange={(event) => changeDraft(event.target.value)}
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
                  disabled={
                    busy ||
                    !!pending ||
                    session?.archived ||
                    recoveryScope === undefined ||
                    (!draft.trim() && !upload.length)
                  }
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
    answers: Array<{
      id: string;
      selected: string[];
      other_text: string | null;
    }>,
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
                    [question.id]: {
                      ...answer,
                      other_text: event.target.value,
                    },
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
