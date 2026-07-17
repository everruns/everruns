"use client";

/**
 * Citation rendering for assistant messages.
 *
 * Consumes the `TextAnnotation`s attached to a message's text (the render
 * contract shared by every citation capability — see `specs/citations.md`) and
 * surfaces them two ways:
 *  - inline numbered chips at each cited span (injected as `[n](#cite-n)`
 *    markers the citation-aware link renderer turns into chips), and
 *  - a deduped "Sources" strip below the message.
 *
 * The renderer is feed-agnostic: it does not care whether a citation came from
 * retrieval, a provider-native feed, or the web — only the shared envelope.
 */

import type { ReactNode } from "react";
import type { TextAnnotation, VerificationVerdict } from "@/lib/api/schema-types";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { ExternalLink, ShieldAlert, ShieldCheck, ShieldQuestion } from "lucide-react";

/** A unique cited source, numbered in first-appearance order. */
export interface CitationSource {
  /** 1-based display number. */
  n: number;
  uri: string;
  title?: string | null;
  snippet?: string | null;
  origin: string;
  verified?: VerificationVerdict | null;
}

/** Result of numbering a message's annotations. */
export interface NumberedCitations {
  /** Deduped sources, ordered by first appearance. */
  sources: CitationSource[];
  /** Source number for each input annotation, parallel to the input array. */
  numberByAnnotation: number[];
}

/**
 * Assign 1-based numbers to unique sources (keyed by `uri`) in first-appearance
 * order, and map each annotation to its source's number. A later annotation for
 * an already-seen uri reuses the number and fills in any missing snippet/verdict.
 */
export function numberCitations(annotations: TextAnnotation[]): NumberedCitations {
  const byUri = new Map<string, CitationSource>();
  const numberByAnnotation: number[] = [];
  for (const ann of annotations) {
    const uri = ann.source.uri;
    let source = byUri.get(uri);
    if (!source) {
      source = {
        n: byUri.size + 1,
        uri,
        title: ann.source.title ?? null,
        snippet: ann.source.snippet ?? null,
        origin: ann.origin,
        verified: ann.verified ?? null,
      };
      byUri.set(uri, source);
    } else {
      source.snippet = source.snippet ?? ann.source.snippet ?? null;
      source.verified = source.verified ?? ann.verified ?? null;
    }
    numberByAnnotation.push(source.n);
  }
  return { sources: [...byUri.values()], numberByAnnotation };
}

/**
 * Insert `[n](#cite-n)` markers at each annotation's end offset so the
 * markdown link renderer can turn them into inline chips. Offsets are char
 * indices into `text`; we splice from the end so earlier offsets stay valid.
 * Out-of-range or overlapping-at-same-offset markers are appended in order.
 */
export function injectCitationMarkers(text: string, annotations: TextAnnotation[]): string {
  const { numberByAnnotation } = numberCitations(annotations);
  const chars = Array.from(text);
  // Insert descending by offset so earlier indices are unaffected.
  const inserts = annotations
    .map((ann, i) => ({ end: ann.end, n: numberByAnnotation[i] }))
    .filter((x) => x.end >= 0 && x.end <= chars.length)
    .sort((a, b) => b.end - a.end);
  for (const { end, n } of inserts) {
    chars.splice(end, 0, `[${n}](#cite-${n})`);
  }
  return chars.join("");
}

/** Parse the `n` out of a `#cite-<n>` href, or null if not a citation link. */
export function parseCitationHref(href?: string): number | null {
  if (!href) return null;
  const match = /^#cite-(\d+)$/.exec(href.trim());
  return match ? Number(match[1]) : null;
}

function hostOf(uri: string): string {
  try {
    return new URL(uri).host;
  } catch {
    return uri;
  }
}

function VerifiedBadge({ verified }: { verified?: VerificationVerdict | null }) {
  if (!verified) return null;
  if (verified.status === "entailed") {
    return (
      <span className="inline-flex items-center gap-0.5 text-[10px] font-medium text-emerald-600 dark:text-emerald-400">
        <ShieldCheck className="h-3 w-3" /> verified
      </span>
    );
  }
  if (verified.status === "unsupported") {
    return (
      <span className="inline-flex items-center gap-0.5 text-[10px] font-medium text-amber-600 dark:text-amber-400">
        <ShieldAlert className="h-3 w-3" /> unsupported
      </span>
    );
  }
  return (
    <span className="inline-flex items-center gap-0.5 text-[10px] font-medium text-muted-foreground">
      <ShieldQuestion className="h-3 w-3" /> unverified
    </span>
  );
}

function SourcePreview({ source }: { source: CitationSource }) {
  const isExternal = /^https?:/i.test(source.uri);
  return (
    <div className="max-w-xs space-y-1">
      <div className="flex items-center justify-between gap-2">
        <span className="text-xs font-medium">{source.title || hostOf(source.uri)}</span>
        <VerifiedBadge verified={source.verified} />
      </div>
      {source.snippet && (
        <p className="line-clamp-4 text-[11px] leading-snug text-muted-foreground">
          {source.snippet}
        </p>
      )}
      <span className="flex items-center gap-1 break-all text-[10px] text-muted-foreground">
        {isExternal && <ExternalLink className="h-2.5 w-2.5 shrink-0" />}
        {source.uri}
      </span>
    </div>
  );
}

/** Inline superscript chip for a cited span. */
export function CitationChip({ source }: { source: CitationSource }): ReactNode {
  const isExternal = /^https?:/i.test(source.uri);
  const chip = (
    <sup
      className="ml-0.5 inline-flex h-4 min-w-4 cursor-pointer items-center justify-center rounded bg-muted px-1 align-super text-[10px] font-medium text-muted-foreground no-underline transition-colors hover:bg-accent hover:text-accent-foreground"
      data-citation={source.n}
    >
      {source.n}
    </sup>
  );
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        {isExternal ? (
          <a href={source.uri} target="_blank" rel="noopener noreferrer">
            {chip}
          </a>
        ) : (
          chip
        )}
      </TooltipTrigger>
      <TooltipContent className="p-2">
        <SourcePreview source={source} />
      </TooltipContent>
    </Tooltip>
  );
}

/** The deduped "Sources" strip rendered below a cited message. */
export function MessageSources({ sources }: { sources: CitationSource[] }): ReactNode {
  if (sources.length === 0) return null;
  return (
    <div className="mt-2 flex flex-wrap items-center gap-1.5 border-t border-border/50 pt-2">
      <span className="text-[10px] uppercase tracking-[0.18em] text-muted-foreground">Sources</span>
      {sources.map((source) => {
        const isExternal = /^https?:/i.test(source.uri);
        const label = (
          <span className="inline-flex items-center gap-1 rounded border border-border/60 bg-muted/60 px-1.5 py-0.5 text-[11px] text-foreground transition-colors hover:bg-accent">
            <span className="font-medium text-muted-foreground">{source.n}</span>
            <span className="max-w-[16rem] truncate">{source.title || hostOf(source.uri)}</span>
            <VerifiedBadge verified={source.verified} />
          </span>
        );
        return (
          <Tooltip key={source.n}>
            <TooltipTrigger asChild>
              {isExternal ? (
                <a href={source.uri} target="_blank" rel="noopener noreferrer">
                  {label}
                </a>
              ) : (
                label
              )}
            </TooltipTrigger>
            <TooltipContent className="p-2">
              <SourcePreview source={source} />
            </TooltipContent>
          </Tooltip>
        );
      })}
    </div>
  );
}
