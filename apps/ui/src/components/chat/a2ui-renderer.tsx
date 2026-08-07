"use client";

/**
 * A2UI Renderer — renders Google A2UI JSON component trees into React.
 *
 * The LLM emits A2UI JSON inside ```a2ui fenced code blocks when the `a2ui`
 * capability is enabled. This renderer walks the JSON tree and dispatches each
 * node to a React component implemented with our own shadcn/ui primitives,
 * so the agent's UI matches the rest of Everruns.
 *
 * Streaming-safe:
 * - Partial JSON parses to a best-effort tree (see parseA2UI).
 * - Unknown types or malformed nodes render a subtle placeholder rather than
 *   throwing.
 * - Wrapped in an error boundary that falls back to the raw JSON code block.
 *
 * @see knowledge/ui/a2ui.md
 */

import {
  Component,
  createContext,
  useContext,
  useState,
  type ErrorInfo,
  type FormEvent,
  type ReactNode,
} from "react";

import { parseA2UI } from "@/lib/a2ui-utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Separator } from "@/components/ui/separator";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";
import { useSessionContext } from "@/app/(main)/sessions/[sessionId]/session-context";
import { useParams } from "next/navigation";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type A2UIAction =
  | { type: "message"; text: string }
  | { type: "open_url"; url: string }
  | { type?: string; [k: string]: unknown };

interface A2UINode {
  type: string;
  props?: Record<string, unknown>;
  children?: A2UINode[];
}

interface A2UIBlockProps {
  /** Raw A2UI JSON (without the ```a2ui fences) */
  code: string;
  /** Whether the LLM is still streaming this content */
  isStreaming?: boolean;
}

// ---------------------------------------------------------------------------
// Error boundary
// ---------------------------------------------------------------------------

interface EBProps {
  children: ReactNode;
  fallback: ReactNode;
}
interface EBState {
  hasError: boolean;
}

class A2UIErrorBoundary extends Component<EBProps, EBState> {
  constructor(props: EBProps) {
    super(props);
    this.state = { hasError: false };
  }
  static getDerivedStateFromError(): EBState {
    return { hasError: true };
  }
  componentDidCatch(error: Error, info: ErrorInfo) {
    console.warn("[a2ui] Render error caught by boundary:", error.message, info.componentStack);
  }
  componentDidUpdate(prevProps: EBProps) {
    if (this.state.hasError && prevProps.children !== this.props.children) {
      // eslint-disable-next-line react/no-did-update-set-state -- ErrorBoundary reset on new children is the standard React pattern
      this.setState({ hasError: false });
    }
  }
  render() {
    return this.state.hasError ? this.props.fallback : this.props.children;
  }
}

// ---------------------------------------------------------------------------
// Action dispatch
// ---------------------------------------------------------------------------

// THREAT[TM-WEB-A2UI-01]: LLM-emitted URLs could use `javascript:`, `data:`, or
// other non-navigational schemes to execute script in the app origin. Restrict
// both `open_url` actions and `Image` src to http/https/mailto.
const SAFE_URL_SCHEMES = ["http:", "https:", "mailto:"] as const;
function isSafeUrl(url: string): boolean {
  try {
    const scheme = new URL(url, window.location.origin).protocol.toLowerCase();
    return (SAFE_URL_SCHEMES as readonly string[]).includes(scheme);
  } catch {
    return false;
  }
}

/**
 * Hook returning an action dispatcher tied to the current session.
 * A2UI blocks only appear inside session-scoped pages, so the session context
 * is guaranteed to be present.
 */
function useActionDispatch(): (action: A2UIAction) => void {
  const ctx = useSessionContext();
  const params = useParams<{ sessionId?: string }>();
  const sessionId = params?.sessionId;

  return (action: A2UIAction) => {
    if (!action || typeof action !== "object") return;
    if (action.type === "message" && typeof action.text === "string" && sessionId) {
      ctx.sendMessage.mutate({ sessionId, content: action.text });
    } else if (
      action.type === "open_url" &&
      typeof action.url === "string" &&
      isSafeUrl(action.url)
    ) {
      window.open(action.url, "_blank", "noopener,noreferrer");
    }
  };
}

// ---------------------------------------------------------------------------
// Prop helpers (safe narrowing for arbitrary LLM output)
// ---------------------------------------------------------------------------

function str(v: unknown, fallback = ""): string {
  return typeof v === "string" ? v : fallback;
}
function optStr(v: unknown): string | undefined {
  return typeof v === "string" ? v : undefined;
}
function bool(v: unknown, fallback = false): boolean {
  return typeof v === "boolean" ? v : fallback;
}
function num(v: unknown): number | undefined {
  return typeof v === "number" && Number.isFinite(v) ? v : undefined;
}
function arr<T = unknown>(v: unknown): T[] {
  return Array.isArray(v) ? (v as T[]) : [];
}
function oneOf<T extends string>(v: unknown, allowed: readonly T[], fallback: T): T {
  return typeof v === "string" && (allowed as readonly string[]).includes(v) ? (v as T) : fallback;
}

// ---------------------------------------------------------------------------
// Form context — collects field values within a <Form>
// ---------------------------------------------------------------------------

interface FormCtx {
  values: Record<string, unknown>;
  set: (name: string, value: unknown) => void;
}

const FormContext = createContext<FormCtx | null>(null);

function useFieldValue<T>(name: string, initial: T): [T, (next: T) => void] {
  const form = useContext(FormContext);
  const [local, setLocal] = useState<T>(initial);
  if (form) {
    const current = (form.values[name] as T | undefined) ?? initial;
    return [current, (next: T) => form.set(name, next)];
  }
  return [local, setLocal];
}

// ---------------------------------------------------------------------------
// Node dispatcher
// ---------------------------------------------------------------------------

function renderNode(
  node: unknown,
  key: number | string,
  dispatch: (a: A2UIAction) => void,
): ReactNode {
  if (!node || typeof node !== "object" || Array.isArray(node)) return null;
  const n = node as A2UINode;
  const type = typeof n.type === "string" ? n.type : "";
  const props = (n.props && typeof n.props === "object" ? n.props : {}) as Record<string, unknown>;
  const children = arr<A2UINode>(n.children);

  const renderChildren = () => children.map((c, i) => renderNode(c, i, dispatch));

  switch (type) {
    // ---------- Layout ----------
    case "Stack": {
      const direction = oneOf(props.direction, ["row", "column"] as const, "column");
      const gap = oneOf(props.gap, ["none", "sm", "md", "lg"] as const, "md");
      const align = oneOf(props.align, ["start", "center", "end", "stretch"] as const, "stretch");
      const gapCls = { none: "gap-0", sm: "gap-2", md: "gap-3", lg: "gap-5" }[gap];
      const dirCls = direction === "row" ? "flex-row flex-wrap" : "flex-col";
      const alignCls =
        direction === "row"
          ? {
              start: "items-start",
              center: "items-center",
              end: "items-end",
              stretch: "items-stretch",
            }[align]
          : {
              start: "items-start",
              center: "items-center",
              end: "items-end",
              stretch: "items-stretch",
            }[align];
      return (
        <div key={key} className={cn("flex", dirCls, gapCls, alignCls)}>
          {renderChildren()}
        </div>
      );
    }

    case "Card": {
      const variant = oneOf(props.variant, ["default", "muted"] as const, "default");
      return (
        <Card key={key} className={cn("px-4", variant === "muted" && "bg-muted/40")}>
          {renderChildren()}
        </Card>
      );
    }

    case "Separator":
      return <Separator key={key} className="my-2" />;

    // ---------- Content ----------
    case "Heading": {
      const level = num(props.level) ?? 3;
      const clamped = Math.min(4, Math.max(1, Math.trunc(level))) as 1 | 2 | 3 | 4;
      const sizeCls = {
        1: "text-xl font-semibold",
        2: "text-lg font-semibold",
        3: "text-base font-semibold",
        4: "text-sm font-semibold",
      }[clamped];
      const cls = cn("text-foreground", sizeCls);
      const text = str(props.text);
      switch (clamped) {
        case 1:
          return (
            <h1 key={key} className={cls}>
              {text}
            </h1>
          );
        case 2:
          return (
            <h2 key={key} className={cls}>
              {text}
            </h2>
          );
        case 3:
          return (
            <h3 key={key} className={cls}>
              {text}
            </h3>
          );
        default:
          return (
            <h4 key={key} className={cls}>
              {text}
            </h4>
          );
      }
    }

    case "Text": {
      const tone = oneOf(props.tone, ["default", "muted"] as const, "default");
      return (
        <p
          key={key}
          className={cn("text-sm leading-relaxed", tone === "muted" && "text-muted-foreground")}
        >
          {str(props.text)}
        </p>
      );
    }

    case "Callout": {
      const variant = oneOf(
        props.variant,
        ["info", "success", "warning", "error"] as const,
        "info",
      );
      const bg = {
        info: "border-blue-500/40 bg-blue-500/10 text-blue-700 dark:text-blue-300",
        success: "border-emerald-500/40 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
        warning: "border-amber-500/40 bg-amber-500/10 text-amber-700 dark:text-amber-300",
        error: "border-red-500/40 bg-red-500/10 text-red-700 dark:text-red-300",
      }[variant];
      const title = optStr(props.title);
      return (
        <div key={key} className={cn("rounded-md border px-3 py-2 text-sm", bg)}>
          {title && <div className="font-medium">{title}</div>}
          <div>{str(props.text)}</div>
        </div>
      );
    }

    case "Badge": {
      const variant = oneOf(
        props.variant,
        ["default", "success", "warning", "error"] as const,
        "default",
      );
      const cls = {
        default: "",
        success: "bg-emerald-500/15 text-emerald-700 dark:text-emerald-300 border-emerald-500/30",
        warning: "bg-amber-500/15 text-amber-700 dark:text-amber-300 border-amber-500/30",
        error: "bg-red-500/15 text-red-700 dark:text-red-300 border-red-500/30",
      }[variant];
      return (
        <Badge key={key} variant="outline" className={cn(cls)}>
          {str(props.label)}
        </Badge>
      );
    }

    case "Image": {
      const src = str(props.src);
      if (!src || !isSafeUrl(src)) return null;
      const caption = optStr(props.caption);
      return (
        <figure key={key} className="flex flex-col gap-1">
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img src={src} alt={str(props.alt, "")} className="max-w-full rounded-md" />
          {caption && <figcaption className="text-xs text-muted-foreground">{caption}</figcaption>}
        </figure>
      );
    }

    case "CodeBlock":
      return (
        <pre key={key} className="overflow-x-auto rounded-md bg-muted p-3 text-xs">
          <code>{str(props.code)}</code>
        </pre>
      );

    // ---------- Data ----------
    case "List": {
      const ordered = bool(props.ordered);
      const listCls = cn("space-y-1 pl-1", ordered ? "list-decimal pl-5" : "list-none");
      return ordered ? (
        <ol key={key} className={listCls}>
          {renderChildren()}
        </ol>
      ) : (
        <ul key={key} className={listCls}>
          {renderChildren()}
        </ul>
      );
    }

    case "ListItem": {
      const title = str(props.title);
      const value = optStr(props.value);
      const description = optStr(props.description);
      return (
        <li key={key} className="flex flex-col gap-0.5 text-sm">
          <div className="flex items-start justify-between gap-2">
            <span className="font-medium">{title}</span>
            {value !== undefined && <span className="text-muted-foreground">{value}</span>}
          </div>
          {description && <span className="text-xs text-muted-foreground">{description}</span>}
        </li>
      );
    }

    case "Table": {
      const columns = arr<unknown>(props.columns).map((c) => str(c));
      const rows = arr<unknown>(props.rows);
      return (
        <Table key={key}>
          <TableHeader>
            <TableRow>
              {columns.map((col, i) => (
                <TableHead key={i}>{col}</TableHead>
              ))}
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((row, ri) => (
              <TableRow key={ri}>
                {arr<unknown>(row).map((cell, ci) => (
                  <TableCell key={ci}>{cellToString(cell)}</TableCell>
                ))}
              </TableRow>
            ))}
          </TableBody>
        </Table>
      );
    }

    // ---------- Forms ----------
    case "Form":
      return (
        <FormNode
          key={key}
          name={str(props.name, "form")}
          submitLabel={str(props.submitLabel, "Submit")}
          submitMessage={optStr(props.submitMessage)}
          dispatch={dispatch}
        >
          {renderChildren()}
        </FormNode>
      );

    case "TextField":
      return (
        <TextFieldNode
          key={key}
          name={str(props.name)}
          label={optStr(props.label)}
          placeholder={optStr(props.placeholder)}
          defaultValue={str(props.defaultValue)}
          required={bool(props.required)}
        />
      );

    case "Textarea":
      return (
        <TextareaNode
          key={key}
          name={str(props.name)}
          label={optStr(props.label)}
          placeholder={optStr(props.placeholder)}
          defaultValue={str(props.defaultValue)}
          rows={num(props.rows)}
        />
      );

    case "Select":
      return (
        <SelectNode
          key={key}
          name={str(props.name)}
          label={optStr(props.label)}
          options={arr<{ label?: unknown; value?: unknown }>(props.options).map((o) => ({
            label: str(o?.label),
            value: str(o?.value),
          }))}
          defaultValue={str(props.defaultValue)}
        />
      );

    case "Checkbox":
      return (
        <CheckboxNode
          key={key}
          name={str(props.name)}
          label={str(props.label)}
          defaultChecked={bool(props.defaultChecked)}
        />
      );

    // ---------- Actions ----------
    case "Button": {
      const action = props.action as A2UIAction | undefined;
      const variant = oneOf(
        props.variant,
        ["primary", "secondary", "ghost", "destructive"] as const,
        "primary",
      );
      const btnVariant =
        variant === "primary" ? "default" : variant === "secondary" ? "secondary" : variant;
      return (
        <Button
          key={key}
          type="button"
          variant={btnVariant}
          size="sm"
          onClick={() => action && dispatch(action)}
          disabled={!action}
        >
          {str(props.label)}
        </Button>
      );
    }

    case "ButtonGroup": {
      const align = oneOf(props.align, ["start", "center", "end"] as const, "start");
      const alignCls = { start: "justify-start", center: "justify-center", end: "justify-end" }[
        align
      ];
      return (
        <div key={key} className={cn("flex flex-wrap gap-2", alignCls)}>
          {renderChildren()}
        </div>
      );
    }

    default:
      // Unknown or missing type — render a subtle placeholder during streaming.
      if (!type) return null;
      return (
        <span
          key={key}
          className="inline-block rounded bg-muted px-2 py-0.5 text-xs text-muted-foreground"
        >
          Unknown component: {type}
        </span>
      );
  }
}

function cellToString(v: unknown): string {
  if (v === null || v === undefined) return "";
  if (typeof v === "string") return v;
  if (typeof v === "number" || typeof v === "boolean") return String(v);
  return "";
}

// ---------------------------------------------------------------------------
// Form node (manages collected values)
// ---------------------------------------------------------------------------

function FormNode({
  name,
  submitLabel,
  submitMessage,
  dispatch,
  children,
}: {
  name: string;
  submitLabel: string;
  submitMessage: string | undefined;
  dispatch: (a: A2UIAction) => void;
  children: ReactNode;
}) {
  const [values, setValues] = useState<Record<string, unknown>>({});
  const ctx: FormCtx = {
    values,
    set: (n, v) => setValues((prev) => ({ ...prev, [n]: v })),
  };

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    const msg = submitMessage ?? defaultSubmitMessage(name, values);
    dispatch({ type: "message", text: msg });
  };

  return (
    <form onSubmit={onSubmit} className="flex flex-col gap-3">
      <FormContext.Provider value={ctx}>{children}</FormContext.Provider>
      <Button type="submit" size="sm" className="self-start">
        {submitLabel}
      </Button>
    </form>
  );
}

function defaultSubmitMessage(name: string, values: Record<string, unknown>): string {
  const entries = Object.entries(values);
  if (entries.length === 0) return `Submitted ${name} form`;
  const pairs = entries.map(([k, v]) => `${k}=${JSON.stringify(v)}`).join(", ");
  return `Submitted ${name} form: ${pairs}`;
}

// ---------------------------------------------------------------------------
// Field nodes
// ---------------------------------------------------------------------------

function TextFieldNode({
  name,
  label,
  placeholder,
  defaultValue,
  required,
}: {
  name: string;
  label: string | undefined;
  placeholder: string | undefined;
  defaultValue: string;
  required: boolean;
}) {
  const [value, setValue] = useFieldValue(name, defaultValue);
  return (
    <div className="flex flex-col gap-1">
      {label && <Label htmlFor={`a2ui-${name}`}>{label}</Label>}
      <Input
        id={`a2ui-${name}`}
        name={name}
        placeholder={placeholder}
        value={value}
        required={required}
        onChange={(e) => setValue(e.target.value)}
      />
    </div>
  );
}

function TextareaNode({
  name,
  label,
  placeholder,
  defaultValue,
  rows,
}: {
  name: string;
  label: string | undefined;
  placeholder: string | undefined;
  defaultValue: string;
  rows: number | undefined;
}) {
  const [value, setValue] = useFieldValue(name, defaultValue);
  return (
    <div className="flex flex-col gap-1">
      {label && <Label htmlFor={`a2ui-${name}`}>{label}</Label>}
      <Textarea
        id={`a2ui-${name}`}
        name={name}
        placeholder={placeholder}
        rows={rows}
        value={value}
        onChange={(e) => setValue(e.target.value)}
      />
    </div>
  );
}

function SelectNode({
  name,
  label,
  options,
  defaultValue,
}: {
  name: string;
  label: string | undefined;
  options: { label: string; value: string }[];
  defaultValue: string;
}) {
  const [value, setValue] = useFieldValue(name, defaultValue);
  return (
    <div className="flex flex-col gap-1">
      {label && <Label htmlFor={`a2ui-${name}`}>{label}</Label>}
      <select
        id={`a2ui-${name}`}
        name={name}
        value={value}
        onChange={(e) => setValue(e.target.value)}
        className="h-8 rounded-md border bg-background px-2 text-sm"
      >
        {!defaultValue && <option value="">Select…</option>}
        {options.map((opt, i) => (
          <option key={i} value={opt.value}>
            {opt.label || opt.value}
          </option>
        ))}
      </select>
    </div>
  );
}

function CheckboxNode({
  name,
  label,
  defaultChecked,
}: {
  name: string;
  label: string;
  defaultChecked: boolean;
}) {
  const [checked, setChecked] = useFieldValue(name, defaultChecked);
  return (
    <label className="flex items-center gap-2 text-sm">
      <Checkbox
        id={`a2ui-${name}`}
        checked={checked}
        onCheckedChange={(v) => setChecked(v === true)}
      />
      <span>{label}</span>
    </label>
  );
}

// ---------------------------------------------------------------------------
// Public block component
// ---------------------------------------------------------------------------

/**
 * Renders a single A2UI JSON block.
 */
export function A2UIBlock({ code, isStreaming }: A2UIBlockProps) {
  if (!code.trim()) return null;

  const fallback = (
    <pre className="overflow-x-auto p-4 text-xs text-muted-foreground">
      <code>{code}</code>
    </pre>
  );

  const tree = parseA2UI(code);

  return (
    <div className="a2ui-block my-2 overflow-hidden rounded-md border border-border bg-card p-3">
      <A2UIErrorBoundary fallback={fallback}>
        <A2UIRoot tree={tree} isStreaming={isStreaming} rawCode={code} />
      </A2UIErrorBoundary>
    </div>
  );
}

function A2UIRoot({
  tree,
  isStreaming,
  rawCode,
}: {
  tree: unknown;
  isStreaming?: boolean;
  rawCode: string;
}) {
  const dispatch = useActionDispatch();
  if (tree === null || tree === undefined) {
    // While streaming, a null tree just means the JSON hasn't arrived yet.
    // Once streaming is complete and parsing still fails, show the raw JSON
    // so users aren't stuck on an indeterminate placeholder.
    if (isStreaming) {
      return <div className="text-xs text-muted-foreground">…</div>;
    }
    return (
      <pre className="overflow-x-auto p-2 text-xs text-muted-foreground">
        <code>{rawCode}</code>
      </pre>
    );
  }
  return <>{renderNode(tree, "root", dispatch)}</>;
}
