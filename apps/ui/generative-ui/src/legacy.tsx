"use client";
import { createElement, useState, type ReactNode } from "react";
import { Streamdown } from "streamdown";
import { parseA2UI } from "./partial";
function safeUrl(value: unknown): value is string {
  if (typeof value !== "string") return false;
  try {
    return ["https:", "http:", "mailto:"].includes(new URL(value).protocol);
  } catch {
    return false;
  }
}
function WidgetForm({
  node,
  onMessage,
  children,
}: {
  node: Node;
  onMessage?: (text: string) => void;
  children: ReactNode;
}) {
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        const values: Record<string, string | boolean> = Object.create(null);
        for (const [name, value] of new FormData(event.currentTarget)) {
          if (typeof value === "string") values[name] = value;
        }
        for (const checkbox of event.currentTarget.querySelectorAll<HTMLInputElement>(
          'input[type="checkbox"][name]',
        )) {
          values[checkbox.name] = checkbox.checked;
        }
        onMessage?.(
          typeof node.props?.submitMessage === "string"
            ? node.props.submitMessage
            : `Submitted ${String(node.props?.name ?? "form")}: ${JSON.stringify(values)}`,
        );
      }}
    >
      {children}
      <button disabled={!onMessage} type="submit">
        {String(node.props?.submitLabel ?? "Submit")}
      </button>
    </form>
  );
}

type Node = { type?: string; props?: Record<string, unknown>; children?: Node[] };
function A2Node({
  node,
  onMessage,
  depth = 0,
}: {
  node: Node;
  onMessage?: (text: string) => void;
  depth?: number;
}): ReactNode {
  const [tab, setTab] = useState(0);
  const [field, setField] = useState<string | boolean>(
    typeof node?.props?.defaultValue === "string"
      ? node.props.defaultValue
      : node?.props?.defaultChecked === true,
  );
  if (!node || typeof node !== "object" || depth > 24) return null;
  const p = node.props ?? {},
    children = (Array.isArray(node.children) ? node.children : []).slice(0, 200);
  const text = (value: unknown) =>
    typeof value === "string" || typeof value === "number" ? String(value) : "";
  const nested = children.map((child, index) => (
    <A2Node key={index} node={child} onMessage={onMessage} depth={depth + 1} />
  ));
  switch (node.type) {
    case "Text":
      return <p>{text(p.text)}</p>;
    case "Heading":
      return createElement(
        `h${Math.min(4, Math.max(1, Math.trunc(Number(p.level) || 3)))}`,
        null,
        text(p.text),
      );
    case "Callout":
      return (
        <aside className="widget-card">
          {typeof p.title === "string" && <strong>{p.title}</strong>}
          <p>{text(p.text)}</p>
          {nested}
        </aside>
      );
    case "CodeBlock":
      return (
        <pre>
          <code>{text(p.code)}</code>
        </pre>
      );
    case "List":
      return p.ordered === true ? <ol>{nested}</ol> : <ul>{nested}</ul>;
    case "ListItem":
      return (
        <li>
          <strong>{text(p.title)}</strong> {text(p.value)}
          {typeof p.description === "string" && <p>{p.description}</p>}
          {nested}
        </li>
      );
    case "Markdown":
      return <Streamdown>{text(p.content ?? p.text)}</Streamdown>;
    case "Badge":
      return <span className="widget-badge">{text(p.label ?? p.text)}</span>;
    case "Separator":
      return <hr />;
    case "Card":
      return (
        <section className="widget-card">
          {typeof p.title === "string" && <h3>{text(p.title)}</h3>}
          {nested}
        </section>
      );
    case "Stack":
    case "Column":
    case "Row":
    case "Grid":
    case "ButtonGroup":
      return (
        <div
          className={
            node.type === "Row" ||
            node.type === "Grid" ||
            node.type === "ButtonGroup" ||
            p.direction === "row"
              ? "widget-row"
              : "widget-stack"
          }
        >
          {nested}
        </div>
      );
    case "Tabs":
      return (
        <div>
          <div role="tablist">
            {children.map((child, index) => (
              <button
                key={index}
                role="tab"
                type="button"
                aria-selected={tab === index}
                onClick={() => setTab(index)}
              >
                {text(child.props?.label ?? child.props?.title) || `Tab ${index + 1}`}
              </button>
            ))}
          </div>
          <div role="tabpanel">{nested[tab]}</div>
        </div>
      );
    case "Tab":
    case "TabItem":
      return <div>{nested}</div>;
    case "Button": {
      const action = p.action as { type?: string; text?: unknown; url?: unknown } | undefined;
      if (action?.type === "open_url" && safeUrl(action.url))
        return (
          <a href={action.url} target="_blank" rel="noopener noreferrer">
            {text(p.label)}
          </a>
        );
      return (
        <button
          type="button"
          disabled={!onMessage || action?.type !== "message" || typeof action.text !== "string"}
          onClick={() => onMessage?.(String(action?.text))}
        >
          {text(p.label)}
        </button>
      );
    }
    case "Image":
      // Generated image URLs are dynamic and intentionally not Next image-loader URLs.
      // oxlint-disable-next-line next/no-img-element
      return safeUrl(p.src) && !String(p.src).startsWith("mailto:") ? (
        // oxlint-disable-next-line next/no-img-element
        <img src={String(p.src)} alt={text(p.alt)} loading="lazy" referrerPolicy="no-referrer" />
      ) : null;
    case "Form":
      return (
        <WidgetForm node={node} onMessage={onMessage}>
          {nested}
        </WidgetForm>
      );
    case "Checkbox": {
      const name = text(p.name);
      return (
        <label>
          <input
            type="checkbox"
            name={name}
            checked={field === true}
            onChange={(event) => {
              setField(event.target.checked);
            }}
          />
          {text(p.label)}
        </label>
      );
    }
    case "TextField":
    case "TextArea":
    case "Textarea":
    case "Input":
    case "Select": {
      const name = text(p.name),
        value = text(field);
      const change = (value: string) => {
        setField(value);
      };
      return (
        <label className="widget-field">
          {text(p.label)}
          {node.type === "Select" ? (
            <select
              name={name}
              aria-label={text(p.label) || name}
              value={value}
              onChange={(event) => change(event.target.value)}
            >
              <option value="">Choose…</option>
              {(Array.isArray(p.options) ? p.options : []).map((option, index) => (
                <option
                  key={index}
                  value={text(
                    typeof option === "object" && option
                      ? (option as Record<string, unknown>).value
                      : option,
                  )}
                >
                  {text(
                    typeof option === "object" && option
                      ? (option as Record<string, unknown>).label
                      : option,
                  )}
                </option>
              ))}
            </select>
          ) : node.type === "TextArea" || node.type === "Textarea" ? (
            <textarea name={name} value={value} onChange={(event) => change(event.target.value)} />
          ) : (
            <input
              name={name}
              required={p.required === true}
              value={value}
              placeholder={text(p.placeholder)}
              onChange={(event) => change(event.target.value)}
            />
          )}
        </label>
      );
    }
    case "Table": {
      const rows = Array.isArray(p.rows) ? p.rows.slice(0, 1000) : [];
      const columns = Array.isArray(p.columns) ? p.columns : [];
      return (
        <div className="widget-table">
          <table>
            <thead>
              <tr>
                {columns.map((column, i) => (
                  <th key={i}>
                    {text(
                      typeof column === "object" && column
                        ? (column as Record<string, unknown>).label
                        : column,
                    )}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((row, i) => (
                <tr key={i}>
                  {(Array.isArray(row)
                    ? row
                    : row && typeof row === "object"
                      ? Object.values(row as Record<string, unknown>)
                      : []
                  )
                    .slice(0, 50)
                    .map((cell, j) => (
                      <td key={j}>{text(cell)}</td>
                    ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    }
    default:
      return (
        <pre>
          <code>{JSON.stringify(node, null, 2)}</code>
        </pre>
      );
  }
}
export function LegacyA2UI({
  code,
  onMessage,
}: {
  code: string;
  onMessage?: (text: string) => void;
}) {
  const tree = parseA2UI(code);
  if (!tree)
    return (
      <pre>
        <code>{code}</code>
      </pre>
    );
  // React render failures are caught by the shared outer Boundary.
  return (
    <div className="legacy-widget">
      <A2Node node={tree} onMessage={onMessage} />
    </div>
  );
}
