"use client";

import { Globe2, type LucideIcon } from "lucide-react";
import { GithubIcon as Github } from "@/components/icons/github-icon";
import { TwitterIcon as Twitter } from "@/components/icons/twitter-icon";
import { type AnchorHTMLAttributes, type ComponentType } from "react";
import { navigationItemForPath } from "@/lib/navigation";
import { cn } from "@/lib/utils";

export type MarkdownLinkKind = "everruns" | "github" | "twitter" | "web";

const SAFE_MARKDOWN_PROTOCOLS = new Set(["http:", "https:", "mailto:", "tel:"]);

const EVERRUNS_HOSTS = new Set([
  "everruns.com",
  "app.everruns.com",
  "dev.everruns.com",
  "docs.everruns.com",
  "staging.everruns.com",
]);

function normalizeHostname(hostname: string): string {
  return hostname.toLowerCase().replace(/^www\./, "");
}

function isEverrunsHost(hostname: string): boolean {
  const normalized = normalizeHostname(hostname);
  return EVERRUNS_HOSTS.has(normalized) || normalized.endsWith(".everruns.com");
}

function getCurrentOrigin(): string | undefined {
  return typeof window === "undefined" ? undefined : window.location.origin;
}

export function isSafeMarkdownHref(href?: string): boolean {
  if (!href) return false;

  try {
    const url = new URL(href.trim(), getCurrentOrigin() ?? "https://everruns.invalid");
    return SAFE_MARKDOWN_PROTOCOLS.has(url.protocol);
  } catch {
    return false;
  }
}

export function getMarkdownLinkKind(href?: string): MarkdownLinkKind | null {
  if (!href) return null;

  const trimmed = href.trim();
  const lower = trimmed.toLowerCase();
  if (
    lower.startsWith("#") ||
    lower.startsWith("mailto:") ||
    lower.startsWith("tel:") ||
    lower.startsWith("javascript:")
  ) {
    return null;
  }

  if (trimmed.startsWith("/") && !trimmed.startsWith("//")) {
    return "everruns";
  }
  if (!/^[a-z][a-z0-9+.-]*:/i.test(trimmed) && !trimmed.startsWith("//")) {
    return "everruns";
  }

  try {
    const url = new URL(trimmed, getCurrentOrigin());
    const host = normalizeHostname(url.hostname);

    if (host === "github.com" || host.endsWith(".github.com")) return "github";
    if (host === "twitter.com" || host.endsWith(".twitter.com")) return "twitter";
    if (host === "x.com" || host.endsWith(".x.com")) return "twitter";
    if (isEverrunsHost(host) || url.origin === getCurrentOrigin()) return "everruns";
    if (url.protocol === "http:" || url.protocol === "https:") return "web";
  } catch {
    return null;
  }

  return null;
}

/** The in-app path an Everruns link points at, or `null` for another Everruns host. */
function everrunsAppPath(href: string): string | null {
  try {
    const origin = getCurrentOrigin() ?? "https://everruns.invalid";
    const url = new URL(href.trim(), origin);
    return url.origin === origin ? url.pathname : null;
  } catch {
    return null;
  }
}

function LinkIcon({ kind, href }: { kind: MarkdownLinkKind; href: string }) {
  if (kind === "everruns") {
    // Links into the app carry the icon of the page they open (an agent link shows the
    // agent icon, a harness link the harness icon); the rings mark only the rest.
    const path = everrunsAppPath(href);
    const NavIcon = path ? navigationItemForPath(path)?.icon : undefined;
    if (NavIcon) return <NavIcon aria-hidden="true" className="markdown-link-icon" />;
    return <span aria-hidden="true" className="markdown-link-icon markdown-link-everruns-icon" />;
  }

  const Icon: LucideIcon = kind === "github" ? Github : kind === "twitter" ? Twitter : Globe2;
  return <Icon aria-hidden="true" className="markdown-link-icon" />;
}

/** Opens markdown links in a new tab and prefixes external links with source icons. */
export const MarkdownLink: ComponentType<AnchorHTMLAttributes<HTMLAnchorElement>> = ({
  children,
  className,
  href,
  ...props
}) => {
  if (!isSafeMarkdownHref(href)) return <span className={className}>{children}</span>;

  const kind = getMarkdownLinkKind(href);

  return (
    <a
      {...props}
      href={href}
      target="_blank"
      rel="noopener noreferrer"
      className={cn(kind && "markdown-link-with-icon", className)}
    >
      {kind && href && <LinkIcon kind={kind} href={href} />}
      {children}
    </a>
  );
};
