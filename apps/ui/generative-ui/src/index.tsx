"use client";
import {
  Component,
  createContext,
  useContext,
  useEffect,
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
  height = 360,
  width = "100%",
}: {
  html: string;
  title?: string;
  height?: number | string;
  width?: number | string;
}) {
  const frame = useRef<HTMLIFrameElement>(null);
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
      srcDoc={`<!doctype html><meta http-equiv="Content-Security-Policy" content="${policy}"><meta name="color-scheme" content="light dark"><style>body{font:14px system-ui;color:${colors.text};background:${colors.background}}</style>${html}`}
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
}: Props) {
  const plan = useMemo(() => buildSurfaces(messages, streaming), [messages, streaming]);
  return (
    <CopilotKitProvider
      runtimeUrl="/generative-ui"
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
