"use client";
import { useCallback, useEffect, useRef, useState } from "react";
import { Streamdown } from "streamdown";

type Config = {
  name: string;
  description?: string;
  interface_mode: "single_with_threads" | "multi_chat";
  features: { threads: boolean; files: boolean };
};
type Question = {
  id: string;
  question: string;
  options?: Array<{ label: string; description?: string }>;
  multi_select?: boolean;
};
type Session = {
  id: string;
  title?: string;
  parent_session_id?: string;
  status: string;
  pending_approvals?: Array<{
    tool_call_id: string;
    answerable: boolean;
    tool_name?: string;
    arguments?: unknown;
  }>;
  pending_questions?: Array<{ tool_call_id: string; questions: Question[] }>;
};
type Event = { id?: string; sequence?: number; type: string; data: Record<string, unknown> };
type Message = { id: string; role: string; text: string };
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
function messageOf(event: Event): Message | null {
  if (!["input.message", "output.message.completed"].includes(event.type)) return null;
  const data = event.data as {
    message?: { id?: string; role?: string; content?: Array<{ type: string; text?: string }> };
    id?: string;
    role?: string;
    content?: Array<{ type: string; text?: string }>;
  };
  const message = data.message ?? data;
  const text = (message.content ?? [])
    .filter((part) => part.type === "text")
    .map((part) => part.text ?? "")
    .join("\n");
  return text
    ? {
        id: message.id ?? event.id ?? "",
        role: message.role ?? (event.type === "input.message" ? "user" : "assistant"),
        text,
      }
    : null;
}
function conversationMessages(events: Event[]): Message[] {
  const messages = new Map<string, Message>();
  for (const event of events) {
    const message = messageOf(event);
    if (message) {
      messages.set(message.id, message);
      continue;
    }
    if (event.type === "output.message.delta") {
      const id = String(event.data.message_id ?? "");
      if (!id) continue;
      const previous = messages.get(id);
      messages.set(id, {
        id,
        role: "assistant",
        text:
          typeof event.data.accumulated === "string"
            ? event.data.accumulated
            : (previous?.text ?? "") + String(event.data.delta ?? ""),
      });
    }
  }
  return [...messages.values()];
}
export function Chat({ config }: { config: Config }) {
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
  const [upload, setUpload] = useState<File | null>(null),
    [sidebar, setSidebar] = useState(false);
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
          next.features.threads !== config.features.threads
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
    const list = await api<{ data: Session[] }>("sessions?limit=100");
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
  const send = async () => {
    if (!draft.trim() && !upload) return;
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
      const attachments = upload
        ? [
            {
              name: upload.name,
              media_type: upload.type || "application/octet-stream",
              data: await new Promise<string>((resolve, reject) => {
                const reader = new FileReader();
                reader.onload = () => resolve(String(reader.result).split(",")[1]);
                reader.onerror = reject;
                reader.readAsDataURL(upload);
              }),
            },
          ]
        : [];
      await api(`sessions/${id}/messages`, "POST", {
        message: { role: "user", content: draft.trim() ? [{ type: "text", text: draft }] : [] },
        files: attachments,
      });
      setDraft("");
      setUpload(null);
    } catch (error) {
      fail(error);
    } finally {
      setBusy(false);
    }
  };
  const browse = async (next: string) => {
    if (!selected) return;
    try {
      const data = await api<{ data?: FileInfo[]; file?: FileContent; revision?: string }>(
        `sessions/${selected}/files?path=${encodeURIComponent(next)}`,
      );
      setError(null);
      if (data.file) {
        setFile({ ...data.file, revision: data.revision });
        setEdit(data.file.content ?? "");
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
  const messages = conversationMessages(events);
  const activity = events.filter(
    (event) => event.type.startsWith("tool.") && event.type !== "tool.call_requested",
  );
  const running = ["active", "started"].includes(session?.status ?? "");
  const choose = (id: string) => {
    const url = new URL(window.location.href);
    url.searchParams.set("session", id);
    window.history.pushState(null, "", url);
    setSelected(id);
    setSidebar(false);
    setWork([]);
  };
  return (
    <div className="app">
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
        <nav aria-label="Conversations">
          {config.interface_mode === "multi_chat"
            ? sessions.map((chat) => (
                <button
                  key={chat.id}
                  className={selected === chat.id ? "row selected" : "row"}
                  onClick={() => choose(chat.id)}
                >
                  {chat.title || "New conversation"}
                </button>
              ))
            : work
                .filter((task) => task.links?.child_session_id)
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
              <Streamdown>{message.text}</Streamdown>
            </article>
          ))}
          {!!activity.length && (
            <details className="activity">
              <summary>
                {running ? "Working" : "Work activity"} · {activity.length} updates
              </summary>
              {activity.slice(-50).map((event, index) => (
                <div key={event.id ?? index}>
                  {String(
                    event.data.narration ??
                      event.data.display_name ??
                      event.data.tool_name ??
                      event.data.activity ??
                      (event.data.tool_call as { name?: string })?.name ??
                      event.data.tool_name ??
                      (event.type === "tool.started" ? "Running a tool" : "Tool completed"),
                  )}
                </div>
              ))}
            </details>
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
              {upload && (
                <div className="attachment">
                  {upload.name}
                  <button
                    type="button"
                    aria-label="Remove attachment"
                    onClick={() => setUpload(null)}
                  >
                    ×
                  </button>
                </div>
              )}
              <textarea
                aria-label="Message"
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
                    onChange={(event) => {
                      const file = event.target.files?.[0];
                      if (file && file.size > 4 * 1024 * 1024) {
                        fail(new Error("Attachments must be under 4 MB"));
                        return;
                      }
                      setUpload(file ?? null);
                    }}
                  />
                </label>
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
                <button className="primary" disabled={busy || (!draft.trim() && !upload)}>
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
                  {file.encoding === "base64" ? (
                    <p>
                      Binary file{" "}
                      <button
                        onClick={() => {
                          const bytes = Uint8Array.from(atob(file.content ?? ""), (char) =>
                            char.charCodeAt(0),
                          );
                          const url = URL.createObjectURL(new Blob([bytes]));
                          const a = document.createElement("a");
                          a.href = url;
                          a.download = file.name;
                          a.click();
                          URL.revokeObjectURL(url);
                        }}
                      >
                        Download
                      </button>
                    </p>
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
  const [answers, setAnswers] = useState<Record<string, string>>({});
  const [sending, setSending] = useState(false);
  return (
    <form
      className="request"
      onSubmit={async (event) => {
        event.preventDefault();
        setSending(true);
        try {
          await onSubmit(
            questions.map((question) => ({
              id: question.id,
              selected: question.options?.length ? [answers[question.id]] : [],
              other_text: question.options?.length ? null : (answers[question.id] ?? ""),
            })),
          );
        } finally {
          setSending(false);
        }
      }}
    >
      {questions.map((question) => (
        <label key={question.id}>
          {question.question}
          {question.options?.length ? (
            <select
              required
              value={answers[question.id] ?? ""}
              onChange={(event) => setAnswers({ ...answers, [question.id]: event.target.value })}
            >
              <option value="">Choose…</option>
              {question.options.map((option) => (
                <option key={option.label} value={option.label}>
                  {option.label}
                </option>
              ))}
            </select>
          ) : (
            <input
              required
              value={answers[question.id] ?? ""}
              onChange={(event) => setAnswers({ ...answers, [question.id]: event.target.value })}
            />
          )}
        </label>
      ))}
      <button disabled={sending}>Continue</button>
    </form>
  );
}
