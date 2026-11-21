"use client";
import {
  Component,
  createContext,
  useContext,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
  type ComponentType,
} from "react";
import { CopilotKitProvider } from "@copilotkit/react-core/v2";
import {
  GenerativeUISurface,
  GenerativeUIError,
  createGenerativeUI,
  standardUI,
  useGenerativeUIRuntime,
  type GenerativeUIComponentProps,
} from "@copilotkit/react-core/v2/generative-ui";
import { Renderer, type ActionEvent } from "@openuidev/react-lang";
import { openuiLibrary } from "@openuidev/react-ui/genui-lib";
import {
  buildSurfaces,
  blockKey,
  chartWithBaseline,
  isLegacy,
  splitUIBlocks,
  type UIMessage,
} from "./protocol";
import { LegacyA2UI } from "./legacy";
export { GenerativeUISurface, standardUI } from "@copilotkit/react-core/v2/generative-ui";
export { splitUIBlocks } from "./protocol";

function SafeHtmlView({
  html,
  title,
  data,
  height = 360,
  width = "100%",
}: {
  html: string;
  title?: string;
  data?: Record<string, unknown>;
  height?: number | string;
  width?: number | string;
}) {
  const frame = useRef<HTMLIFrameElement>(null);
  const staticHtml = useMemo(() => {
    if (typeof document === "undefined") return "";
    const template = document.createElement("template");
    template.innerHTML = html;
    // THREAT[TM-WEB-024]: sandboxing blocks script and top navigation, but an
    // iframe can still navigate itself. Keep generated HTML a static preview.
    for (const node of template.content.querySelectorAll("meta, base")) node.remove();
    for (const node of template.content.querySelectorAll("a, area")) {
      node.removeAttribute("href");
      node.removeAttribute("xlink:href");
    }
    if (data) {
      const walker = document.createTreeWalker(template.content, NodeFilter.SHOW_TEXT);
      for (let node = walker.nextNode(); node; node = walker.nextNode()) {
        if (node.parentElement?.closest("style, script")) continue;
        node.textContent = (node.textContent ?? "").replace(
          /\{\{([a-zA-Z0-9_]+)\}\}/g,
          (token, key: string) => {
            if (!Object.hasOwn(data, key)) return token;
            const value = data[key];
            return typeof value === "string" ? value : (JSON.stringify(value) ?? "");
          },
        );
      }
    }
    return template.innerHTML;
  }, [html, data]);
  const [colors, setColors] = useState({ text: "CanvasText", background: "Canvas" });
  useEffect(() => {
    const sync = () => {
      if (!frame.current) return;
      const style = getComputedStyle(frame.current);
      const text = style.getPropertyValue("--genui-text").trim();
      const background = style.getPropertyValue("--genui-bg").trim();
      if (CSS.supports("color", text) && CSS.supports("color", background))
        setColors({ text, background });
    };
    sync();
    const observer = new MutationObserver(sync);
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class", "style"],
    });
    const media = matchMedia("(prefers-color-scheme: dark)");
    media.addEventListener("change", sync);
    return () => {
      observer.disconnect();
      media.removeEventListener("change", sync);
    };
  }, []);
  const policy =
    "default-src 'none'; img-src data:; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'";
  return (
    <iframe
      ref={frame}
      sandbox=""
      title={title ?? "HTML"}
      srcDoc={`<!doctype html><meta http-equiv="Content-Security-Policy" content="${policy}"><meta name="color-scheme" content="light dark"><style>body{font:14px system-ui;color:${colors.text};background:${colors.background}}</style>${staticHtml}`}
      style={{
        display: "block",
        width,
        height,
        border: "1px solid var(--genui-border)",
        borderRadius: 8,
      }}
    />
  );
}

export type AgentViewLayout =
  | { format: "a2ui"; operations: unknown[] }
  | { format: "html"; source: string };

export type PublishedAgentView = {
  revision: number;
  title: string;
  layout: AgentViewLayout;
  data: Record<string, unknown>;
  personal_layout: AgentViewLayout | null;
  allow_personalization: boolean;
  updated_at: string | null;
};

export function AgentViewSurface({
  title,
  layout,
  data,
}: {
  title: string;
  layout: AgentViewLayout;
  data?: Record<string, unknown>;
}) {
  const id = useId();
  if (layout.format === "html") {
    return (
      <div className="everruns-widget widget">
        <SafeHtmlView html={layout.source} title={title} data={data} />
      </div>
    );
  }
  const first = layout.operations[0] as
    | undefined
    | {
        createSurface?: { surfaceId?: string; components?: unknown[] };
      };
  const surfaceId = first?.createSurface?.surfaceId;
  const base = layout.operations.slice();
  if (first?.createSurface && !first.createSurface.components) {
    // The pinned renderer expects the initial component list on createSurface.
    // Accept layouts that send their first component list as a separate update.
    const updates = base as Array<{ updateComponents?: { components?: unknown[] } } | null>;
    const components = updates.find((operation) =>
      Array.isArray(operation?.updateComponents?.components),
    )?.updateComponents?.components;
    if (components) base[0] = { ...first, createSurface: { ...first.createSurface, components } };
  }
  const operations =
    surfaceId && data
      ? [
          ...base,
          ...Object.entries(data).map(([key, value]) => ({
            version: "1.0",
            updateDataModel: { surfaceId, path: "/" + key, value },
          })),
        ]
      : base;
  const text = "```a2ui\n" + JSON.stringify(operations) + "\n```";
  const messages = [{ id, text }];
  return (
    <GenerativeUIConversation messages={messages}>
      <MessageWidgets text={text} messageId={id} renderMarkdown={() => null} />
    </GenerativeUIConversation>
  );
}
export function PublishedView({
  view,
  saveLayout,
  translate = (key) => key,
  locale,
}: {
  view: PublishedAgentView;
  saveLayout?: (revision: number, layout: AgentViewLayout | null) => Promise<unknown>;
  translate?: (key: string) => string;
  locale?: string;
}) {
  const id = useId();
  const [editing, setEditing] = useState(false);
  const [source, setSource] = useState("");
  const [format, setFormat] = useState<AgentViewLayout["format"]>("a2ui");
  const [preview, setPreview] = useState<AgentViewLayout | null>(null);
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);
  const layout = view.personal_layout ?? view.layout;
  const canCustomize = view.allow_personalization && !!saveLayout;

  function edit() {
    setFormat(layout.format);
    setSource(
      layout.format === "html" ? layout.source : JSON.stringify(layout.operations, null, 2),
    );
    setPreview(null);
    setError("");
    setEditing(true);
  }

  function draft(): AgentViewLayout | undefined {
    if (format === "html" && source.trim()) return { format, source };
    if (format === "a2ui") {
      try {
        const operations: unknown = JSON.parse(source);
        if (Array.isArray(operations) && operations.length) return { format, operations };
      } catch {
        /* Fall through to the editor error. */
      }
    }
    setError(
      format === "html"
        ? translate("Enter HTML markup.")
        : translate("Enter a JSON array of A2UI operations."),
    );
  }

  async function save(next: AgentViewLayout | null) {
    if (!saveLayout) return;
    setSaving(true);
    setError("");
    try {
      await saveLayout(view.revision, next);
      setEditing(false);
      setPreview(null);
    } catch (error) {
      setError(error instanceof Error ? error.message : translate("Could not save your layout."));
    } finally {
      setSaving(false);
    }
  }

  return (
    <section className="everruns-published-view" aria-labelledby={id}>
      <header>
        <div>
          <h2 id={id}>{view.title}</h2>
          {view.updated_at && (
            <time dateTime={view.updated_at}>
              {new Date(view.updated_at).toLocaleString(locale)}
            </time>
          )}
        </div>
        {canCustomize && !editing && (
          <button type="button" onClick={edit}>
            {translate("Customize")}
          </button>
        )}
      </header>
      <AgentViewSurface
        title={view.title}
        layout={canCustomize && editing ? (preview ?? layout) : layout}
        data={view.data}
      />
      {canCustomize && editing && (
        <div className="everruns-view-preference">
          <label htmlFor={id + "-format"}>{translate("Layout format")}</label>
          <select
            id={id + "-format"}
            value={format}
            disabled={saving}
            onChange={(event) => setFormat(event.target.value === "html" ? "html" : "a2ui")}
          >
            <option value="a2ui">{translate("Generated UI (A2UI)")}</option>
            <option value="html">{translate("Custom HTML")}</option>
          </select>
          <label htmlFor={id + "-source"}>{translate("Personal layout source")}</label>
          <textarea
            id={id + "-source"}
            value={source}
            disabled={saving}
            spellCheck={false}
            onChange={(event) => {
              setSource(event.target.value);
              setError("");
            }}
          />
          {error && <p role="alert">{error}</p>}
          <div>
            <button
              type="button"
              disabled={saving}
              onClick={() => {
                const next = draft();
                if (next) {
                  setPreview(next);
                  setError("");
                }
              }}
            >
              {translate("Preview draft")}
            </button>
            <button
              type="button"
              disabled={saving}
              onClick={() => {
                const next = draft();
                if (next) void save(next);
              }}
            >
              {translate("Save for me")}
            </button>
            {view.personal_layout && (
              <button type="button" disabled={saving} onClick={() => void save(null)}>
                {translate("Use published layout")}
              </button>
            )}
            <button type="button" disabled={saving} onClick={() => setEditing(false)}>
              {translate("Cancel")}
            </button>
          </div>
        </div>
      )}
    </section>
  );
}

type StandardProps<
  Name extends keyof NonNullable<typeof standardUI.definitions>["components"] & string,
> = GenerativeUIComponentProps<typeof standardUI, Name>;
const StandardButton = standardUI.components.get("Button")!.component as ComponentType<
  StandardProps<"Button">
>;
const StandardTextField = standardUI.components.get("TextField")!.component as ComponentType<
  StandardProps<"TextField">
>;
const StandardTextArea = standardUI.components.get("TextArea")!.component as ComponentType<
  StandardProps<"TextArea">
>;
const StandardSelect = standardUI.components.get("Select")!.component as ComponentType<
  StandardProps<"Select">
>;
const StandardForm = standardUI.components.get("Form")!.component as ComponentType<
  StandardProps<"Form">
>;
const StandardChart = standardUI.components.get("Chart")!.component as ComponentType<
  StandardProps<"Chart">
>;
function ChatButton(props: StandardProps<"Button">) {
  return <StandardButton size="md" pill={false} {...props} />;
}
function ChatTextField(props: StandardProps<"TextField">) {
  return <StandardTextField size="md" pill={false} {...props} />;
}
function ChatTextArea(props: StandardProps<"TextArea">) {
  return <StandardTextArea size="md" {...props} />;
}
function ChatSelect(props: StandardProps<"Select">) {
  return <StandardSelect size="md" pill={false} {...props} />;
}
function ChatForm(props: StandardProps<"Form">) {
  return <StandardForm align="start" {...props} />;
}
function ChatChart(props: StandardProps<"Chart">) {
  return <StandardChart {...chartWithBaseline(props)} />;
}

// Generated HTML keeps the same no-script/no-network contract as workspace previews.
const chatUI = createGenerativeUI({
  id: "everruns/chat@1",
  extends: [standardUI],
  components: {},
  overrides: {
    HtmlView: { component: SafeHtmlView },
    Button: { component: ChatButton },
    TextField: { component: ChatTextField },
    TextArea: { component: ChatTextArea },
    Select: { component: ChatSelect },
    Form: { component: ChatForm },
    Chart: { component: ChatChart },
  },
});
const theme = {
  "--a2ui-color-text-default": "var(--genui-text)",
  "--a2ui-color-text-prose": "var(--genui-text)",
  "--a2ui-color-text-secondary": "var(--genui-muted)",
  "--a2ui-color-text-tertiary": "var(--genui-muted)",
  "--a2ui-color-surface": "var(--genui-bg)",
  "--a2ui-color-panel": "var(--genui-panel)",
  "--a2ui-color-control": "var(--genui-panel)",
  "--a2ui-color-border-default": "var(--genui-border)",
  "--a2ui-color-border-control": "var(--genui-border)",
  "--a2ui-color-accent": "var(--genui-accent)",
  "--a2ui-color-on-accent": "var(--genui-on-accent)",
  "--a2ui-font-family": "inherit",
};

type SendMessage = (text: string) => void | Promise<unknown>;
type Props = {
  messages: readonly UIMessage[];
  streaming?: boolean;
  onMessage?: SendMessage;
  children: ReactNode;
  runtimeUrl?: string;
};
type Plan = ReturnType<typeof buildSurfaces>;
const Conversation = createContext<(Plan & { onMessage?: SendMessage; streaming: boolean }) | null>(
  null,
);

// The provider discovers no agents. Everruns retains transport and authentication;
// only the standard component/surface engine runs here.
export function GenerativeUIConversation({
  messages,
  streaming = false,
  onMessage,
  children,
  runtimeUrl = "/generative-ui",
}: Props) {
  const plan = useMemo(() => buildSurfaces(messages, streaming), [messages, streaming]);
  return (
    <CopilotKitProvider
      runtimeUrl={runtimeUrl}
      useSingleEndpoint={false}
      enableInspector={false}
      generativeUI={{ ui: chatUI, theme, transport: "markdown", sendContext: false }}
    >
      <Conversation.Provider value={{ ...plan, onMessage, streaming }}>
        <Bridge plan={plan} onMessage={onMessage} streaming={streaming} />
        {children}
      </Conversation.Provider>
    </CopilotKitProvider>
  );
}
function Bridge({
  plan,
  onMessage,
  streaming,
}: {
  plan: Plan;
  onMessage?: SendMessage;
  streaming: boolean;
}) {
  const runtime = useGenerativeUIRuntime();
  const pending = useRef(false);
  useEffect(() => {
    if (!runtime) return;
    const previous = runtime.forwardEvent;
    // The pinned renderer exposes this transport callback as a mutable adapter.
    // Everruns owns it; the provider never runs an agent.
    // oxlint-disable-next-line react/immutability
    runtime.forwardEvent = async (event) => {
      if (!onMessage || streaming || pending.current)
        return {
          ok: false,
          error: new GenerativeUIError("agent_busy", "This conversation is read-only or busy"),
        };
      pending.current = true;
      try {
        const context =
          event.context && Object.keys(event.context).length
            ? `\n${JSON.stringify(event.context)}`
            : "";
        await onMessage(`${event.userMessage || event.name}${context}`);
        return { ok: true, result: undefined };
      } catch {
        return {
          ok: false,
          error: new GenerativeUIError("event_forward_failed", "The message could not be sent"),
        };
      } finally {
        pending.current = false;
      }
    };
    return () => {
      runtime.forwardEvent = previous;
    };
  }, [runtime, onMessage, streaming]);
  useEffect(() => {
    if (!runtime) return;
    runtime.manager.setActiveThread("conversation");
    for (const surface of plan.surfaces.values())
      runtime.manager.ingest("conversation", "default", surface);
  }, [runtime, plan]);
  return null;
}

class Boundary extends Component<{ children: ReactNode; code: string }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() {
    return { failed: true };
  }
  componentDidUpdate(previous: { code: string }) {
    // A recovered streaming block must be allowed to render again.
    // oxlint-disable-next-line react/no-did-update-set-state
    if (this.state.failed && previous.code !== this.props.code) this.setState({ failed: false });
  }
  render() {
    return this.state.failed ? (
      <pre>
        <code>{this.props.code}</code>
      </pre>
    ) : (
      this.props.children
    );
  }
}
function openUIAction(event: ActionEvent, send: SendMessage) {
  if (event.type === "open_url") {
    const url = event.params.url;
    if (typeof url !== "string") return;
    try {
      if (["https:", "http:", "mailto:"].includes(new URL(url).protocol))
        window.open(url, "_blank", "noopener,noreferrer");
    } catch {
      /* Invalid generated URLs have no action. */
    }
    return;
  }
  // Every server action follows the existing message/approval path. The
  // renderer receives no Query/Mutation tool provider.
  const state =
    event.formState && Object.keys(event.formState).length
      ? `\n${JSON.stringify(event.formState)}`
      : "";
  const context = event.params.context ? `\n${JSON.stringify(event.params.context)}` : "";
  return send((event.humanFriendlyMessage || event.type) + state + context);
}
const noSubscription = () => () => {};
const noVersion = () => 0;
function Surface({
  target,
  code,
  fallbackLabel,
}: {
  target: string;
  code: string;
  fallbackLabel: string;
}) {
  const runtime = useGenerativeUIRuntime();
  useSyncExternalStore(
    runtime?.manager.subscribe ?? noSubscription,
    runtime?.manager.getVersion ?? noVersion,
    noVersion,
  );
  const surface =
    runtime?.manager.getSurfaceForTarget(target) ??
    (target.startsWith("chat:")
      ? runtime?.manager.getFailedSurface("conversation", target.slice(5))
      : undefined);
  const rejected = [...(surface?.clientRejected ?? []), ...(surface?.serverRejected ?? [])];
  return (
    <>
      <GenerativeUISurface id={target} autoScroll={false} />
      {!!rejected.length && (
        <div role="alert">
          <details>
            <summary>{fallbackLabel}</summary>
            <pre>
              <code>{code}</code>
            </pre>
          </details>
        </div>
      )}
    </>
  );
}

export function MessageWidgets(props: {
  text: string;
  messageId: string;
  streaming?: boolean;
  onMessage?: SendMessage;
  fallbackLabel?: string;
  sendErrorLabel?: string;
  renderMarkdown: (text: string, key: string) => ReactNode;
}) {
  const context = useContext(Conversation);
  const pending = useRef(false);
  const [sending, setSending] = useState(false);
  const [actionError, setActionError] = useState("");
  if (!context)
    return (
      <GenerativeUIConversation
        messages={[{ id: props.messageId, text: props.text }]}
        streaming={props.streaming}
        onMessage={props.onMessage}
      >
        <MessageWidgets {...props} />
      </GenerativeUIConversation>
    );
  const send = props.onMessage ?? context.onMessage;
  const busy = props.streaming || context.streaming || sending;
  const fallbackLabel = props.fallbackLabel ?? "This content could not be displayed.";
  const sendAction = async (text: string) => {
    if (!send || busy || pending.current || !text.trim()) return;
    pending.current = true;
    setSending(true);
    setActionError("");
    try {
      await send(text);
    } catch {
      setActionError(props.sendErrorLabel ?? "The message could not be sent.");
    } finally {
      pending.current = false;
      setSending(false);
    }
  };
  return (
    <>
      {splitUIBlocks(props.text).map((segment) => {
        const key = blockKey(props.messageId, segment.index);
        if (segment.type === "markdown") return props.renderMarkdown(segment.content, key);
        if (segment.content.length > 256 * 1024)
          return (
            <p key={key} role="alert">
              {fallbackLabel}
            </p>
          );
        const readonly = !send || busy;
        return (
          <Boundary key={key} code={segment.content}>
            <div className="everruns-widget widget" data-readonly={readonly || undefined}>
              <fieldset disabled={readonly} aria-busy={busy || undefined}>
                {segment.type === "openui" ? (
                  <Renderer
                    response={segment.content}
                    library={openuiLibrary}
                    isStreaming={!!busy}
                    onAction={
                      send && !busy
                        ? (event) => {
                            void openUIAction(event, sendAction);
                          }
                        : undefined
                    }
                  />
                ) : isLegacy(segment.content) ? (
                  <LegacyA2UI
                    code={segment.content}
                    onMessage={
                      !readonly
                        ? (text) => {
                            void sendAction(text);
                          }
                        : undefined
                    }
                  />
                ) : (
                  <>
                    {context.mounts.get(key)?.map((target) => (
                      <Surface
                        key={target}
                        target={target}
                        code={segment.content}
                        fallbackLabel={fallbackLabel}
                      />
                    ))}
                    {context.errors.has(key) && (
                      <div role="alert">
                        <details>
                          <summary>{fallbackLabel}</summary>
                          <pre>
                            <code>{segment.content}</code>
                          </pre>
                        </details>
                      </div>
                    )}
                  </>
                )}
              </fieldset>
            </div>
          </Boundary>
        );
      })}
      {actionError && <p role="alert">{actionError}</p>}
    </>
  );
}
