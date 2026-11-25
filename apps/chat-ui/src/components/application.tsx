"use client";
import { useEffect, useRef, useState } from "react";
import { usePathname } from "next/navigation";
import { Chat } from "./chat";

type Resource = { slug: string; name: string; kind: "chat" | "room" | "memory"; writable: boolean };
type Manifest = { name: string; description: string; resources: Resource[] };
async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`/api/application/${path}`, { cache: "no-store", ...init });
  if (response.status === 401 || response.status === 403) throw new Error("Sign in to continue");
  if (!response.ok)
    throw new Error(
      response.status === 409
        ? "This file already exists. Reload before editing."
        : "The request could not be completed",
    );
  return response.status === 204 ? (undefined as T) : response.json();
}
const group = { chat: "Chats", room: "Rooms", memory: "Memory" };
const href = (r: Resource) =>
  `/${r.kind === "chat" ? "chats" : r.kind === "room" ? "rooms" : "memory"}/${r.slug}`;
export function Application({ config }: { config: { name: string; description: string } }) {
  const pathname = usePathname();
  const [manifest, setManifest] = useState<Manifest>();
  const [error, setError] = useState("");
  useEffect(() => {
    request<Manifest>("manifest")
      .then(setManifest)
      .catch((e) => setError(e.message));
  }, []);
  const selected = manifest?.resources.find((r) => href(r) === pathname);
  return (
    <div className="application-shell">
      <header className="application-header">
        <a className="application-wordmark" href="/">
          <span className="application-mark">{config.name.slice(0, 1)}</span>
          {config.name}
        </a>
        <nav aria-label="Application navigation">
          <a href="/" aria-current={pathname === "/" ? "page" : undefined}>
            Overview
          </a>
          {Object.entries(group).map(([kind, label]) => {
            const resources = manifest?.resources.filter((r) => r.kind === kind) ?? [];
            return resources.length > 0 ? (
              <label key={kind} className="application-nav-group">
                <span>{label}</span>
                <select
                  aria-label={label}
                  value={selected?.kind === kind ? href(selected) : ""}
                  onChange={(e) => {
                    if (e.target.value) location.assign(e.target.value);
                  }}
                >
                  <option value="">Choose…</option>
                  {resources.map((r) => (
                    <option key={r.slug} value={href(r)}>
                      {r.name}
                    </option>
                  ))}
                </select>
              </label>
            ) : null;
          })}
        </nav>
        {manifest && (
          <button
            onClick={async () => {
              await fetch("/auth/logout", { method: "POST" });
              location.reload();
            }}
          >
            Sign out
          </button>
        )}
      </header>
      {error ? (
        <section className="application-welcome">
          <span className="application-eyebrow">Welcome to {config.name}</span>
          <h1>A place to work together.</h1>
          <p>{config.description}</p>
          <p role="alert">{error}</p>
          <a className="primary" href={`/auth/start?returnTo=${encodeURIComponent(pathname)}`}>
            Continue to sign in →
          </a>
        </section>
      ) : !manifest ? (
        <p className="application-loading" role="status">
          Opening application…
        </p>
      ) : selected?.kind === "chat" ? (
        <ApplicationChat resource={selected} />
      ) : selected?.kind === "room" ? (
        <ApplicationRoom key={selected.slug} resource={selected} />
      ) : selected?.kind === "memory" ? (
        <ApplicationMemory key={selected.slug} resource={selected} />
      ) : pathname !== "/" ? (
        <section className="application-welcome">
          <h1>This resource is unavailable</h1>
          <a href="/">Back to overview</a>
        </section>
      ) : (
        <section className="application-welcome">
          <span className="application-eyebrow">Your workspace</span>
          <h1>{manifest.name}</h1>
          <p>{manifest.description || "Choose a chat, join a room, or explore shared memory."}</p>
          {Object.entries(group).map(([kind, label]) => {
            const resources = manifest.resources.filter((r) => r.kind === kind);
            return resources.length > 0 ? (
              <section key={kind} className="application-section">
                <h2>{label}</h2>
                <div className="application-grid">
                  {resources.map((r) => (
                    <a className="application-card" key={r.slug} href={href(r)}>
                      <span className="application-card-icon" aria-hidden>
                        {kind === "chat" ? "↗" : kind === "room" ? "#" : "≡"}
                      </span>
                      <strong>{r.name}</strong>
                      <span>
                        {kind === "chat"
                          ? "Start a conversation"
                          : kind === "room"
                            ? "Work with people and agents"
                            : r.writable
                              ? "Read and edit shared files"
                              : "Explore shared knowledge"}
                      </span>
                    </a>
                  ))}
                </div>
              </section>
            ) : null;
          })}
        </section>
      )}
    </div>
  );
}
function ApplicationChat({ resource }: { resource: Resource }) {
  const [config, setConfig] = useState<Parameters<typeof Chat>[0]["config"]>();
  const [error, setError] = useState("");
  useEffect(() => {
    fetch(`/api/chats/${resource.slug}/chat/config`, { cache: "no-store" })
      .then(async (response) => {
        if (!response.ok) throw new Error("This chat is unavailable");
        return response.json();
      })
      .then(setConfig)
      .catch((e) => setError(e.message));
  }, [resource.slug]);
  return error ? (
    <p role="alert">{error}</p>
  ) : config ? (
    <Chat key={resource.slug} config={{ ...config, name: resource.name }} />
  ) : (
    <p role="status">Opening chat…</p>
  );
}
type RoomMessage = {
  id: string;
  text: string;
  author?: { display_name: string; handle: string };
  created_at: string;
  reply_count: number;
  deleted: boolean;
};
function ApplicationRoom({ resource }: { resource: Resource }) {
  const [info, setInfo] = useState<{
    topic?: string;
    members: { id: string; handle: string; display_name: string }[];
  }>();
  const [messages, setMessages] = useState<RoomMessage[]>([]),
    [more, setMore] = useState(false),
    [thread, setThread] = useState<RoomMessage>(),
    [text, setText] = useState(""),
    [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  const loadedEarlier = useRef(false);
  const base = `rooms/${resource.slug}`;
  const route = thread ? `${base}/replies/${thread.id}` : `${base}/messages`;
  useEffect(() => {
    request<typeof info>(base)
      .then(setInfo)
      .catch((e) => setError(e.message));
  }, [base]);
  useEffect(() => {
    let active = true;
    loadedEarlier.current = false;
    setMessages([]);
    const refresh = () =>
      request<{ data: RoomMessage[]; has_more: boolean }>(route)
        .then((data) => {
          if (active) {
            setMessages((previous) => {
              if (!loadedEarlier.current || !data.data.length) return data.data;
              const first = data.data[0].created_at;
              return [...previous.filter((m) => m.created_at < first), ...data.data];
            });
            if (!loadedEarlier.current) setMore(data.has_more);
          }
        })
        .catch((e) => {
          if (active) setError(e.message);
        });
    void refresh();
    const timer = setInterval(refresh, 4000);
    return () => {
      active = false;
      clearInterval(timer);
    };
  }, [route]);
  async function send(e: React.FormEvent) {
    e.preventDefault();
    if (!text.trim() || busy) return;
    setBusy(true);
    setError("");
    try {
      await request(`${base}/messages`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ text, reply_to: thread?.id }),
      });
      setText("");
      const data = await request<{ data: RoomMessage[]; has_more: boolean }>(route);
      setMessages((previous) =>
        loadedEarlier.current && data.data.length
          ? [...previous.filter((m) => m.created_at < data.data[0].created_at), ...data.data]
          : data.data,
      );
      if (!loadedEarlier.current) setMore(data.has_more);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="application-room">
      <header>
        <span className="application-eyebrow">Room</span>
        <h1>{resource.name}</h1>
        <p>{info?.topic}</p>
        <div className="application-members">
          {info?.members.map((m) => (
            <span key={m.id} title={m.display_name}>
              @{m.handle}
            </span>
          ))}
        </div>
      </header>
      {thread && (
        <div className="application-thread">
          <button onClick={() => setThread(undefined)}>← All messages</button>
          <p>{thread.text}</p>
        </div>
      )}
      {error && <p role="alert">{error}</p>}
      <div className="application-messages" aria-live="polite">
        {more && (
          <button
            onClick={async () => {
              try {
                const data = await request<{ data: RoomMessage[]; has_more: boolean }>(
                  `${route}?before=${encodeURIComponent(messages[0].created_at)}`,
                );
                loadedEarlier.current = true;
                setMessages((current) => [
                  ...data.data.filter((m) => !current.some((existing) => existing.id === m.id)),
                  ...current,
                ]);
                setMore(data.has_more);
              } catch (e) {
                setError((e as Error).message);
              }
            }}
          >
            Load earlier messages
          </button>
        )}
        {messages.length === 0 && (
          <p className="application-empty">
            No messages yet. Start the conversation, or mention an agent using @handle.
          </p>
        )}
        {messages.map((m) => (
          <article key={m.id} className="application-message">
            <header>
              <strong>{m.author?.display_name ?? "Room"}</strong>
              <time dateTime={m.created_at}>{new Date(m.created_at).toLocaleString()}</time>
            </header>
            <p>{m.deleted ? "Message deleted" : m.text}</p>
            {!thread && !m.deleted && (
              <button onClick={() => setThread(m)}>
                {m.reply_count ? `${m.reply_count} replies` : "Reply"}
              </button>
            )}
          </article>
        ))}
      </div>
      <form onSubmit={send} className="application-composer">
        <label htmlFor="room-message">{thread ? "Reply to thread" : "Message the room"}</label>
        <textarea
          id="room-message"
          value={text}
          onChange={(e) => setText(e.target.value)}
          maxLength={40000}
          rows={3}
          placeholder="Write a message. Mention @handle to invite an agent."
        />
        <button className="primary" disabled={busy || !text.trim()}>
          {busy ? "Sending…" : "Send message"}
        </button>
      </form>
    </section>
  );
}
type MemoryFile = { path: string; is_directory: boolean; size_bytes: number };
function ApplicationMemory({ resource }: { resource: Resource }) {
  const [directory, setDirectory] = useState("/"),
    [files, setFiles] = useState<MemoryFile[]>([]),
    [selected, setSelected] = useState<string>(),
    [content, setContent] = useState(""),
    [newName, setNewName] = useState(""),
    [error, setError] = useState(""),
    [busy, setBusy] = useState(false),
    [saved, setSaved] = useState(false);
  const base = `memory/${resource.slug}`;
  const reload = () =>
    request<{ files: MemoryFile[] }>(`${base}?path=${encodeURIComponent(directory)}`).then((data) =>
      setFiles(data.files),
    );
  useEffect(() => {
    void reload().catch((e) => setError(e.message));
  }, [base, directory]);
  async function open(file: MemoryFile) {
    setError("");
    setSaved(false);
    if (file.is_directory) {
      setDirectory(file.path);
      setSelected(undefined);
      return;
    }
    try {
      const data = await request<{ content: string }>(
        `${base}?path=${encodeURIComponent(file.path)}&file=true`,
      );
      setSelected(file.path);
      setContent(data.content);
    } catch (e) {
      setError((e as Error).message);
    }
  }
  async function write(create: boolean) {
    setBusy(true);
    setError("");
    setSaved(false);
    try {
      const path = create ? `${directory.replace(/\/$/, "")}/${newName}` : selected;
      if (!path) return;
      await request(base, {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ path, content: create ? "" : content, create }),
      });
      setNewName("");
      setSelected(path);
      if (create) setContent("");
      setSaved(true);
      await reload();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="application-memory">
      <header>
        <span className="application-eyebrow">
          Shared memory · {resource.writable ? "Read and write" : "Read only"}
        </span>
        <h1>{resource.name}</h1>
        <p>Files shared with this application’s people and agents.</p>
      </header>
      {error && <p role="alert">{error}</p>}
      <div className="application-memory-browser">
        <aside>
          <div className="application-path">
            <button
              disabled={directory === "/"}
              onClick={() => {
                setDirectory(directory.slice(0, directory.lastIndexOf("/")) || "/");
                setSelected(undefined);
              }}
            >
              ↑
            </button>
            <code>{directory}</code>
          </div>
          {files.length === 0 && <p>No files in this folder.</p>}
          {files.map((f) => (
            <button
              key={f.path}
              className={selected === f.path ? "selected" : ""}
              onClick={() => void open(f)}
            >
              {f.is_directory ? "▸ " : ""}
              {f.path.split("/").pop()}
            </button>
          ))}
          {resource.writable && (
            <form
              onSubmit={(e) => {
                e.preventDefault();
                void write(true);
              }}
            >
              <label htmlFor="new-file">New file</label>
              <input
                id="new-file"
                value={newName}
                onChange={(e) => setNewName(e.target.value)}
                placeholder="notes.md"
                maxLength={200}
              />
              <button disabled={busy || !newName.trim()}>Create file</button>
            </form>
          )}
        </aside>
        <div className="application-memory-editor">
          {selected ? (
            <>
              <h2>{selected}</h2>
              <textarea
                aria-label="File content"
                readOnly={!resource.writable}
                value={content}
                onChange={(e) => {
                  setContent(e.target.value);
                  setSaved(false);
                }}
                spellCheck={false}
              />
              {resource.writable && (
                <div className="application-actions">
                  <button className="primary" disabled={busy} onClick={() => void write(false)}>
                    Save changes
                  </button>
                  <button
                    disabled={busy}
                    onClick={async () => {
                      if (!confirm(`Delete ${selected}?`)) return;
                      setBusy(true);
                      try {
                        await request(`${base}?path=${encodeURIComponent(selected)}`, {
                          method: "DELETE",
                        });
                        setSelected(undefined);
                        await reload();
                      } catch (e) {
                        setError((e as Error).message);
                      } finally {
                        setBusy(false);
                      }
                    }}
                  >
                    Delete file
                  </button>
                  {saved && <span role="status">Saved</span>}
                </div>
              )}
            </>
          ) : (
            <p className="application-empty">
              Select a file to read{resource.writable ? " or edit" : ""}.
            </p>
          )}
        </div>
      </div>
    </section>
  );
}
