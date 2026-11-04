// Illustrated social cards (og:image) for changelog entries and blog posts.
//
// One renderer for two sites: the docs build (generate-og-image.mjs) uses it for
// changelog entries, and everruns/landing keeps a byte-for-byte copy for blog
// posts (`npm run sync:social-card -- <everruns checkout>` there). Keep it free of
// dependencies: it returns SVG strings, and each site rasterizes them its own way
// (sharp here, resvg-wasm on landing). Every font is Geist or Geist Mono.
//
// Layout: text column on the left (mark, gold eyebrow, title, meta lines, URL),
// illustration stage on the right. What fills the stage depends on how much the
// entry matters (knowledge/docs/changelog.md, "Social cards"):
//   - `art`: an agent-drawn social.svg, a 520x560 drawing placed as is;
//   - `icons`: a drawing generated from the app's Facet domain icons, a hero icon
//     in a chamfered gold frame wired to a few related icons;
//   - nothing: a plain text card (minor entries).
// The generated drawing is seeded by the slug, so a card never changes between
// builds unless its inputs do.

export const W = 1200;
export const H = 630;
const PAD = 72;

export const THEME = {
  bg: "#0A1636",
  title: "#FFFFFF",
  text: "#A8B0BE",
  muted: "#7D879A",
  gold: "#D4A43A",
  line: "#3A4766",
  dot: "rgba(168,176,190,0.22)",
  panel: "#0F1D45",
  soft: "rgba(168,176,190,0.16)",
};

/** Where an agent-drawn social.svg lands: its own 520x560 coordinate space. */
export const ART_STAGE = { x: 650, y: 35, w: 520, h: 560 };

const esc = (s) =>
  String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

// ─── Facet icons ────────────────────────────────────────────────────────────

/**
 * Reads the Facet masters out of apps/ui/src/components/icons/facet-icon-data.ts,
 * so the cards draw the exact icons the app shows. The file is a trusted,
 * repository-owned object literal.
 */
export function parseFacetIcons(tsSource) {
  const match = tsSource.match(/export const facetIconData = (\{[\s\S]*?\n\}) satisfies/);
  if (!match) throw new Error("facetIconData not found in facet-icon-data.ts");
  return new Function(`return (${match[1]});`)();
}

function facet(icons, name, x, y, size, color, strokeWidth = 1.75) {
  const data = icons[name];
  if (!data) return "";
  const scale = size / 24;
  const paint = data.solid ? `fill="${color}" stroke="none"` : `fill="none" stroke="${color}" stroke-width="${strokeWidth}"`;
  return `<g transform="translate(${x} ${y}) scale(${scale})" ${paint} stroke-linecap="square" stroke-linejoin="miter">${data.paths
    .map((d) => `<path d="${d}"/>`)
    .join("")}</g>`;
}

// Topic → icons. First match on the title, description and docs links wins.
// Satellites are the related domains drawn around the hero.
const TOPICS = [
  [/avatar|branding/, "agent", ["account", "exposure", "chat"]],
  [/observer|evals?\b|grader|judge|quality|pass rate/, "observer", ["evals", "health", "session"]],
  [/sandbox|daytona|\be2b\b|modal|desktop|computer use|browser/, "sandbox", ["sandboxTemplate", "worker", "schedule"]],
  [/schedul|cron|trigger|proactive/, "schedule", ["queue", "agent", "durable"]],
  [/workflow/, "workflow", ["queue", "durable", "agent"]],
  [/durable|actor|resum|crash|drain|retr(y|ies)/, "durable", ["worker", "queue", "session"]],
  [/worker|deploy|helm|kubernetes|docker|scal|load/, "worker", ["durable", "health", "queue"]],
  [/cost|budget|billing|usage|spend|credit|price/, "payments", ["report", "organization", "agent"]],
  [/trace|monitor|metric|dashboard|timeline|doctor|health|incident/, "report", ["health", "session", "worker"]],
  [/model|provider|openrouter|anthropic|openai|mistral|gemini|llm|cach/, "models", ["provider", "token", "agent"]],
  [/skill/, "skills", ["agent", "capabilities", "knowledge"]],
  [/plugin/, "plugins", ["capabilities", "agent", "skills"]],
  [/\bmcp\b|tool|capabilit|web search/, "capabilities", ["plugins", "agent", "exposure"]],
  [/memory/, "memory", ["knowledge", "session", "agent"]],
  [/knowledge|search|docs/, "knowledge", ["memory", "skills", "agent"]],
  [/api key|token|auth|secret|sign|permission|scope|oauth|egress|login/, "token", ["account", "organization", "agent"]],
  [/organi[sz]ation|team|member|invit|user/, "organization", ["team", "account", "settings"]],
  [/slack|channel|a2a|ag-ui|webhook|expos|endpoint|\bapi\b|sdk/, "exposure", ["session", "agent", "chat"]],
  [/chat|message|conversation|reaction|voice/, "chat", ["session", "agent", "exposure"]],
  [/session|attachment|file/, "session", ["chat", "agent", "memory"]],
  [/harness/, "harness", ["agent", "capabilities", "models"]],
  [/playground/, "playground", ["agent", "session", "evals"]],
  [/setting|config/, "settings", ["agent", "organization", "features"]],
  [/agent/, "agent", ["session", "exposure", "capabilities"]],
];

const KIND_FALLBACK = {
  release: ["durable", ["worker", "workflow", "agent", "provider"]],
  fix: ["health", ["session", "worker"]],
  breaking: ["settings", ["features", "agent"]],
  deprecation: ["schedule", ["features", "agent"]],
  incident: ["health", ["report", "worker"]],
};

/**
 * Picks the hero icon and satellites for an entry or post. Explicit frontmatter
 * (`icons: [hero, ...satellites]`) wins over keyword matching.
 */
export function pickIcons({ icons: explicit, title = "", description = "", links = [], kind = "" }, available) {
  const valid = (name) => typeof name === "string" && name in available;
  if (Array.isArray(explicit) && explicit.length > 0 && explicit.every(valid)) {
    return { hero: explicit[0], satellites: explicit.slice(1, 5) };
  }
  if (kind === "release") {
    const [hero, satellites] = KIND_FALLBACK.release;
    return { hero, satellites };
  }
  const haystack = `${title} ${description} ${links.join(" ")}`.toLowerCase();
  for (const [pattern, hero, satellites] of TOPICS) {
    if (pattern.test(haystack)) return { hero, satellites: satellites.filter(valid) };
  }
  const [hero, satellites] = KIND_FALLBACK[kind] ?? ["agent", ["session", "exposure", "capabilities"]];
  return { hero, satellites };
}

// ─── Generated illustration ─────────────────────────────────────────────────

function rng(seed) {
  let h = 2166136261;
  for (const c of String(seed)) h = Math.imul(h ^ c.charCodeAt(0), 16777619);
  return () => {
    h ^= h << 13;
    h ^= h >>> 17;
    h ^= h << 5;
    return ((h >>> 0) % 10000) / 10000;
  };
}

function dotGrid(t) {
  return `<defs><radialGradient id="sc-fade" cx="0.62" cy="0.5" r="0.55"><stop offset="0" stop-color="#fff" stop-opacity="1"/><stop offset="1" stop-color="#fff" stop-opacity="0"/></radialGradient>
  <mask id="sc-mask"><rect x="560" y="0" width="${W - 560}" height="${H}" fill="url(#sc-fade)"/></mask>
  <pattern id="sc-dots" width="20" height="20" patternUnits="userSpaceOnUse"><circle cx="2" cy="2" r="1.4" fill="${t.dot}"/></pattern></defs>
  <rect x="560" y="0" width="${W - 560}" height="${H}" fill="url(#sc-dots)" mask="url(#sc-mask)"/>`;
}

/**
 * The hero icon (or a product mark) in a chamfered gold frame on a run timeline,
 * with up to four related icons wired to it. `mark` is an optional product logo
 * SVG (512x512) drawn instead of the hero icon. `glass` makes the panels
 * translucent for cards drawn over a photo.
 */
export function iconIllustration({ seed, hero, satellites = [], mark, icons, glass = false, t = THEME }) {
  const r = rng(seed);
  const fx = 790;
  const fy = 150;
  const fw = 300;
  const fh = 300;
  const c = 22;
  const cx = fx + fw / 2;
  const cy = fy + fh / 2;
  let s = "";

  // Run timeline: ticks across the stage, tallest near the frame.
  for (let x = 700; x < W - 12; x += 9) {
    if (x > fx - 16 && x < fx + fw + 16) continue;
    const near = 1 - Math.min(1, Math.abs(x - cx) / 520);
    const height = 6 + r() * 110 * near * near;
    const gold = r() < 0.22;
    s += `<rect x="${x}" y="${(cy - height / 2).toFixed(1)}" width="2" height="${height.toFixed(1)}" fill="${gold ? t.gold : t.line}" opacity="${gold ? 0.9 : 0.75}"/>`;
  }

  // Satellites in the corners, wired to the frame with orthogonal connectors.
  const slots = [
    { x: 690, y: 70 },
    { x: 690, y: 470 },
    { x: 1110, y: 470 },
    { x: 1110, y: 70 },
  ];
  satellites.slice(0, slots.length).forEach((name, i) => {
    const p = slots[i];
    const size = 64;
    const top = p.y;
    const fromY = p.y < cy ? top + size : top;
    const edgeY = p.y < cy ? fy : fy + fh;
    const toX = p.x < cx ? fx + 60 : fx + fw - 60;
    s += `<path d="M${p.x} ${fromY} V${(fromY + edgeY) / 2} H${toX} V${edgeY}" fill="none" stroke="${t.line}" stroke-width="2" stroke-dasharray="6 6"/>`;
    s += `<rect x="${toX - 4}" y="${edgeY - 4}" width="8" height="8" fill="${t.gold}"/>`;
    s += `<rect x="${p.x - size / 2}" y="${top}" width="${size}" height="${size}" fill="${t.panel}" fill-opacity="${glass ? 0.85 : 1}" stroke="${t.line}" stroke-width="2"/>`;
    s += facet(icons, name, p.x - size / 2 + 14, top + 14, 36, t.text);
  });

  // The frame: chamfered panel, dashed inner border, a panel header strip.
  const frame = `M${fx + c} ${fy} H${fx + fw} V${fy + fh - c} L${fx + fw - c} ${fy + fh} H${fx} V${fy + c} Z`;
  s += `<path d="${frame}" fill="${t.panel}" fill-opacity="${glass ? 0.82 : 1}" stroke="${t.gold}" stroke-width="3"/>`;
  s += `<rect x="${fx + 16}" y="${fy + 16}" width="${fw - 32}" height="${fh - 32}" fill="none" stroke="${t.gold}" stroke-opacity="0.55" stroke-width="2" stroke-dasharray="8 7"/>`;
  for (let i = 0; i < 3; i++) {
    s += `<rect x="${fx + 32 + i * 16}" y="${fy + 32}" width="9" height="9" fill="${i === 0 ? t.gold : t.line}"/>`;
  }
  s += `<rect x="${fx + 92}" y="${fy + 33}" width="${(70 + r() * 90).toFixed(0)}" height="7" fill="${t.soft}"/>`;
  if (mark) {
    s += `<svg x="${fx + 60}" y="${fy + 66}" width="180" height="180" viewBox="0 0 512 512">${goldMark(mark, t)}</svg>`;
  } else {
    s += facet(icons, hero, fx + 60, fy + 66, 180, t.gold, 1.2);
  }
  for (const [x, y] of [
    [fx - 14, fy - 14],
    [fx + fw + 6, fy + fh + 6],
  ]) {
    s += `<rect x="${x}" y="${y}" width="8" height="8" fill="${t.gold}"/>`;
  }
  return s;
}

/** A product logo (black or navy strokes on 512x512) recolored to gold. */
function goldMark(svg, t) {
  const root = svg.match(/<svg[^>]*>/)?.[0] ?? "";
  const fill = root.match(/\sfill="([^"]*)"/)?.[1];
  const inner = svg.replace(/^[\s\S]*?<svg[^>]*>/, "").replace(/<\/svg>\s*$/, "");
  return (fill ? `<g fill="${fill}">${inner}</g>` : inner).replace(/#0A0A0A|#0A1636/gi, t.gold).replace(/id="([^"]+)"/g, 'id="sc-$1"').replace(/url\(#/g, "url(#sc-");
}

// ─── Agent-drawn art ────────────────────────────────────────────────────────

/** Problems with an agent-drawn social.svg, as messages. Empty means it is usable. */
export function validateArt(svg) {
  const problems = [];
  const root = svg.match(/<svg\b[^>]*>/);
  if (!root) return ["no <svg> root element"];
  if (!/viewBox="0 0 520 560"/.test(root[0])) problems.push('the root <svg> needs viewBox="0 0 520 560"');
  if (/<script|<foreignObject|<iframe|<object|<embed/i.test(svg)) problems.push("no script, foreignObject or embedded documents");
  if (/\son[a-z]+\s*=/i.test(svg)) problems.push("no event handler attributes");
  if (/<style/i.test(svg) && /@import/i.test(svg)) problems.push("no @import in styles");
  for (const [, ref] of svg.matchAll(/(?:xlink:)?href\s*=\s*"([^"]*)"/g)) {
    if (ref.startsWith("#")) continue;
    if (/^[a-z][a-z0-9+.-]*:/i.test(ref) || ref.startsWith("//")) {
      problems.push(`${ref.slice(0, 60)}: images must be repository files referenced by relative path`);
    } else if (!/\.(png|jpe?g|webp|svg)$/i.test(ref)) {
      problems.push(`${ref}: images must be PNG, JPEG, WebP or SVG`);
    }
  }
  return problems;
}

/** Relative image references in an agent-drawn social.svg. */
export function artImageRefs(svg) {
  return [...new Set([...svg.matchAll(/(?:xlink:)?href\s*=\s*"([^"#][^"]*)"/g)].map((m) => m[1]))];
}

/**
 * Places an agent-drawn social.svg on the stage. `images` maps each relative
 * reference to a data URI, so the card carries no outside references.
 */
export function placeArt(svg, images = {}) {
  let out = svg.replace(/<\?xml[^>]*>/, "").trim();
  for (const [ref, uri] of Object.entries(images)) {
    out = out.split(`"${ref}"`).join(`"${uri}"`);
  }
  // Keep ids from colliding with the card's own.
  out = out.replace(/\bid="([^"]+)"/g, 'id="art-$1"').replace(/url\(#/g, "url(#art-").replace(/href="#/g, 'href="#art-');
  return out.replace(
    /<svg\b([^>]*)>/,
    (_, attrs) =>
      `<svg${attrs.replace(/\s(x|y|width|height)="[^"]*"/g, "")} x="${ART_STAGE.x}" y="${ART_STAGE.y}" width="${ART_STAGE.w}" height="${ART_STAGE.h}">`,
  );
}

// ─── The card ───────────────────────────────────────────────────────────────

function wrap(text, maxChars, maxLines) {
  const lines = [];
  let current = "";
  for (const word of String(text).split(/\s+/).filter(Boolean)) {
    if (current && current.length + 1 + word.length > maxChars) {
      lines.push(current);
      current = word;
    } else {
      current = current ? `${current} ${word}` : word;
    }
  }
  if (current) lines.push(current);
  if (lines.length > maxLines) {
    lines.length = maxLines;
    lines[maxLines - 1] = `${lines[maxLines - 1].replace(/\s*\S*$/, "")}…`;
  }
  return lines;
}

function brandMark(x, y, size, t) {
  const s = size / 512;
  return `<g transform="translate(${x} ${y}) scale(${s})" fill="none" stroke-width="22" stroke-linecap="round">
    <circle cx="256" cy="196" r="120" stroke="${t.gold}"/><circle cx="186" cy="291" r="120" stroke="${t.gold}" opacity="0.7"/><circle cx="326" cy="291" r="120" stroke="${t.gold}" opacity="0.7"/></g>`;
}

/**
 * A complete 1200x630 card as SVG.
 *
 * - `eyebrow`, `title`, `meta` (lines), `url`: the text column.
 * - `description`: shown under the title on plain cards only.
 * - `illustration`: SVG for the right half (iconIllustration or placeArt), or
 *   empty for a plain card.
 * - `photo`: a data URI drawn full bleed under a left-heavy navy gradient (blog).
 */
export function socialCardSvg({ eyebrow, title, description = "", meta = [], url, illustration = "", photo, t = THEME }) {
  const plain = !illustration;
  const columnChars = plain ? 38 : 20;
  const size = title.length > 60 ? 48 : title.length > 36 ? 54 : 62;
  const lineHeight = Math.round(size * 1.12);
  const lines = wrap(title, Math.floor((columnChars * 52) / size), plain ? 3 : 4);
  const descLines = plain && description ? wrap(description, 64, lines.length > 2 ? 2 : 3) : [];
  const blockHeight = (lines.length - 1) * lineHeight + (descLines.length ? 52 + (descLines.length - 1) * 36 + 56 : 58);
  const titleTop = Math.round(330 - blockHeight / 2);
  const afterTitle = titleTop + (lines.length - 1) * lineHeight;
  const metaTop = afterTitle + (descLines.length ? 52 + (descLines.length - 1) * 36 + 56 : 58);
  const background = photo
    ? `<defs><linearGradient id="sc-overlay" x1="0" x2="1" y1="0" y2="0"><stop offset="0" stop-color="${t.bg}" stop-opacity="0.94"/><stop offset="0.45" stop-color="${t.bg}" stop-opacity="0.86"/><stop offset="1" stop-color="${t.bg}" stop-opacity="0.55"/></linearGradient></defs>
  <image href="${photo}" x="0" y="0" width="${W}" height="${H}" preserveAspectRatio="xMidYMid slice"/>
  <rect width="${W}" height="${H}" fill="url(#sc-overlay)"/>`
    : "";
  return `<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}">
  <rect width="${W}" height="${H}" fill="${t.bg}"/>
  ${background}
  ${plain ? "" : dotGrid(t)}
  ${illustration}
  ${brandMark(PAD - 4, 54, 42, t)}
  <text x="${PAD + 48}" y="84" font-family="Geist" font-size="25" font-weight="600" letter-spacing="-0.4" fill="${t.title}">Everruns</text>
  <text x="${PAD}" y="${titleTop - lineHeight - 4}" font-family="Geist Mono" font-size="18" font-weight="500" letter-spacing="2.5" fill="${t.gold}">${esc(String(eyebrow).toUpperCase())}</text>
  ${lines
    .map(
      (line, i) =>
        `<text x="${PAD}" y="${titleTop + i * lineHeight}" font-family="Geist" font-size="${size}" font-weight="600" letter-spacing="${-size * 0.02}" fill="${t.title}" xml:space="preserve">${esc(line)}</text>`,
    )
    .join("\n  ")}
  ${descLines
    .map(
      (line, i) =>
        `<text x="${PAD}" y="${afterTitle + 52 + i * 36}" font-family="Geist" font-size="25" fill="${t.text}" xml:space="preserve">${esc(line)}</text>`,
    )
    .join("\n  ")}
  ${meta
    .filter(Boolean)
    .map(
      (line, i) =>
        `<text x="${PAD}" y="${metaTop + i * 30}" font-family="Geist Mono" font-size="19" fill="${t.text}" xml:space="preserve">${esc(line)}</text>`,
    )
    .join("\n  ")}
  <text x="${PAD}" y="${H - 50}" font-family="Geist Mono" font-size="18" fill="${t.muted}">${esc(url)}</text>
</svg>`;
}
