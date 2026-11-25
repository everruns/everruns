/**
 * HTML hooks for the Words Markdown renderer (`@pierre/words`).
 *
 * Decisions:
 * - Words hands each generated element to a hook as an HTML string and takes an HTML
 *   string back; it supports no remark/rehype plugins. Everything the console adds on
 *   top of plain GFM (GitHub alerts, link handling, citation chips) lives here as pure
 *   string transforms so it is unit-testable without a DOM.
 * - React-only decorations (link source icons, citation chips with their tooltip) are
 *   emitted as empty `data-md-slot` placeholders that `MarkdownSlots` fills with portals.
 * - Hooks are trusted code: every value written into markup is either a fixed string,
 *   a number, or an attribute Words already escaped.
 */

import { getMarkdownLinkKind, isSafeMarkdownHref } from "@/components/markdown/markdown-link";
import { parseCitationHref } from "@/components/chat/message-citations";

const ALERT_KINDS = ["note", "tip", "important", "warning", "caution"] as const;
type AlertKind = (typeof ALERT_KINDS)[number];

// Octicons, as remark-github-blockquote-alert drew them (MIT).
const ALERT_ICON_PATHS: Record<AlertKind, string> = {
  note: "M0 8a8 8 0 1 1 16 0A8 8 0 0 1 0 8Zm8-6.5a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13ZM6.5 7.75A.75.75 0 0 1 7.25 7h1a.75.75 0 0 1 .75.75v2.75h.25a.75.75 0 0 1 0 1.5h-2a.75.75 0 0 1 0-1.5h.25v-2h-.25a.75.75 0 0 1-.75-.75ZM8 6a1 1 0 1 1 0-2 1 1 0 0 1 0 2Z",
  tip: "M8 1.5c-2.363 0-4 1.69-4 3.75 0 .984.424 1.625.984 2.304l.214.253c.223.264.47.556.673.848.284.411.537.896.621 1.49a.75.75 0 0 1-1.484.211c-.04-.282-.163-.547-.37-.847a8.456 8.456 0 0 0-.542-.68c-.084-.1-.173-.205-.268-.32C3.201 7.75 2.5 6.766 2.5 5.25 2.5 2.31 4.863 0 8 0s5.5 2.31 5.5 5.25c0 1.516-.701 2.5-1.328 3.259-.095.115-.184.22-.268.319-.207.245-.383.453-.541.681-.208.3-.33.565-.37.847a.751.751 0 0 1-1.485-.212c.084-.593.337-1.078.621-1.489.203-.292.45-.584.673-.848.075-.088.147-.173.213-.253.561-.679.985-1.32.985-2.304 0-2.06-1.637-3.75-4-3.75ZM5.75 12h4.5a.75.75 0 0 1 0 1.5h-4.5a.75.75 0 0 1 0-1.5ZM6 15.25a.75.75 0 0 1 .75-.75h2.5a.75.75 0 0 1 0 1.5h-2.5a.75.75 0 0 1-.75-.75Z",
  important:
    "M0 1.75C0 .784.784 0 1.75 0h12.5C15.216 0 16 .784 16 1.75v9.5A1.75 1.75 0 0 1 14.25 13H8.06l-2.573 2.573A1.458 1.458 0 0 1 3 14.543V13H1.75A1.75 1.75 0 0 1 0 11.25Zm1.75-.25a.25.25 0 0 0-.25.25v9.5c0 .138.112.25.25.25h2a.75.75 0 0 1 .75.75v2.19l2.72-2.72a.749.749 0 0 1 .53-.22h6.5a.25.25 0 0 0 .25-.25v-9.5a.25.25 0 0 0-.25-.25Zm7 2.25v2.5a.75.75 0 0 1-1.5 0v-2.5a.75.75 0 0 1 1.5 0ZM9 9a1 1 0 1 1-2 0 1 1 0 0 1 2 0Z",
  warning:
    "M6.457 1.047c.659-1.234 2.427-1.234 3.086 0l6.082 11.378A1.75 1.75 0 0 1 14.082 15H1.918a1.75 1.75 0 0 1-1.543-2.575Zm1.763.707a.25.25 0 0 0-.44 0L1.698 13.132a.25.25 0 0 0 .22.368h12.164a.25.25 0 0 0 .22-.368Zm.53 3.996v2.5a.75.75 0 0 1-1.5 0v-2.5a.75.75 0 0 1 1.5 0ZM9 11a1 1 0 1 1-2 0 1 1 0 0 1 2 0Z",
  caution:
    "M4.47.22A.749.749 0 0 1 5 0h6c.199 0 .389.079.53.22l4.25 4.25c.141.14.22.331.22.53v6a.749.749 0 0 1-.22.53l-4.25 4.25A.749.749 0 0 1 11 16H5a.749.749 0 0 1-.53-.22L.22 11.53A.749.749 0 0 1 0 11V5c0-.199.079-.389.22-.53Zm.84 1.28L1.5 5.31v5.38l3.81 3.81h5.38l3.81-3.81V5.31L10.69 1.5ZM8 4a.75.75 0 0 1 .75.75v3.5a.75.75 0 0 1-1.5 0v-3.5A.75.75 0 0 1 8 4Zm0 8a1 1 0 1 1 0-2 1 1 0 0 1 0 2Z",
};

const ALERT_TITLES: Record<AlertKind, string> = {
  note: "Note",
  tip: "Tip",
  important: "Important",
  warning: "Warning",
  caution: "Caution",
};

const ALERT_OPENING =
  /^<blockquote([^>]*)>(\s*)<p([^>]*)>\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\][ \t]*(?:\n|<br\s*\/?>\s*)?/i;

/**
 * `> [!NOTE]` blockquotes become GitHub-style alerts, with the same classes and icon the
 * previous remark plugin produced so existing styles keep applying.
 */
export function alertBlockquoteHook(html: string): string {
  const match = ALERT_OPENING.exec(html);
  if (!match) return html;
  const [opening, bqAttrs, space, pAttrs, rawKind] = match;
  const kind = rawKind.toLowerCase() as AlertKind;
  const icon = `<svg class="octicon" viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><path d="${ALERT_ICON_PATHS[kind]}"></path></svg>`;
  const title = `<p class="markdown-alert-title" dir="auto">${icon}${ALERT_TITLES[kind]}</p>`;
  const rest = html.slice(opening.length);
  // An alert whose first paragraph held only the marker leaves an empty paragraph behind.
  const body = `<p${pAttrs}>${rest}`.replace(/^<p[^>]*>\s*<\/p>\s*/, "");
  return `<div${bqAttrs.replace(/\sdata-markdown="blockquote"/, "")} class="markdown-alert markdown-alert-${kind}" dir="auto">${space}${title}${body.replace(/<\/blockquote>\s*$/, "</div>")}`;
}

function decodeAttribute(value: string): string {
  return value
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&#(\d+);/g, (_, n: string) => String.fromCodePoint(Number(n)))
    .replace(/&#x([0-9a-f]+);/gi, (_, n: string) => String.fromCodePoint(parseInt(n, 16)))
    .replace(/&amp;/g, "&");
}

const LINK = /^<a\b([^>]*)>([\s\S]*)<\/a>$/;

/**
 * Links: in-page anchors (footnotes) stay as they are; `#cite-<n>` markers become citation
 * chip slots when the message has citations; other safe links open in a new tab and get a
 * source-icon slot; unsafe links render as plain text.
 */
export function createLinkHook(citationNumbers?: ReadonlySet<number>) {
  return (html: string): string => {
    const match = LINK.exec(html.trim());
    if (!match) return html;
    const [, attrs, inner] = match;
    const hrefMatch = /\shref="([^"]*)"/.exec(attrs);
    const href = hrefMatch ? decodeAttribute(hrefMatch[1]) : undefined;

    if (citationNumbers && citationNumbers.size > 0) {
      const n = parseCitationHref(href);
      if (n !== null) {
        return citationNumbers.has(n) ? `<span data-md-slot="cite" data-n="${n}"></span>` : "";
      }
    }
    if (href?.trim().startsWith("#")) return html;
    if (!isSafeMarkdownHref(href)) return `<span>${inner}</span>`;

    const kind = getMarkdownLinkKind(href);
    const iconSlot = kind ? `<span data-md-slot="link-icon" data-kind="${kind}"></span>` : "";
    const linkClass = kind ? ' class="markdown-link-with-icon"' : "";
    return `<a${attrs} target="_blank" rel="noopener noreferrer"${linkClass}>${iconSlot}${inner}</a>`;
  };
}
