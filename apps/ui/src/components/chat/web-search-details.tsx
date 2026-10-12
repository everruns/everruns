"use client";

// One web search, for people: the "Searched '…'" row in chat and the drawer
// it opens (Details, Raw, Event). Both read the people-only `details` the
// `web_search` tool puts on its result, so they show the provider that
// answered, every attempt and each provider's verbatim response, whatever the
// model was told. See knowledge/integrations/web-search.md (Details view).

import { useState } from "react";
import {
  AlertCircle,
  ChevronDown,
  ChevronRight,
  Download,
  ExternalLink,
  Search,
} from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { CopyButton } from "@/components/ui/copy-button";
import {
  Drawer,
  DrawerContent,
  DrawerDescription,
  DrawerHeader,
  DrawerTitle,
} from "@/components/ui/drawer";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { JsonView, formatBytes } from "@/components/session/trace/json-view";
import { SearchProviderIcon } from "@/components/web-search/search-provider-icon";
import {
  attemptStatus,
  formatSearchDuration,
  settingLabel,
  siteOf,
  summarizeWebSearch,
  type SearchAttempt,
  type WebSearchDetails,
} from "@/lib/web-search";
import { cn } from "@/lib/utils";

/** Raw bodies show this much until "Show all"; the rest is one click or a download away. */
const RAW_PREVIEW_BYTES = 2048;
const INLINE_RESULTS = 3;

function providerKind(provider?: string): "native" | "free" | "account" {
  if (provider === "native") return "native";
  if (provider === "free") return "free";
  return "account";
}

function accountProvider(provider?: string): string | undefined {
  if (provider === "brave" || provider?.startsWith("account:")) return "brave_search";
  return undefined;
}

/** Collapsed and expanded chat row for one completed `web_search` call. */
export function WebSearchRow({
  details,
  durationMs,
  event,
}: {
  details: WebSearchDetails;
  durationMs?: number | null;
  /** The tool.completed payload, for the drawer's Event tab. */
  event?: unknown;
}) {
  const [expanded, setExpanded] = useState(false);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const summary = summarizeWebSearch(details, durationMs);
  const shown = details.results.slice(0, INLINE_RESULTS);
  const more = details.results.length - shown.length;

  return (
    <div
      data-testid="web-search-row"
      className={cn("my-1 border bg-card text-sm", summary.fallbackNote && "border-warning/40")}
    >
      <div
        className={cn(
          "flex min-w-0 items-center gap-2 px-3 py-2",
          summary.fallbackNote && "bg-warning/8",
        )}
      >
        {summary.failed ? (
          <AlertCircle className="size-3.5 shrink-0 text-destructive" aria-hidden="true" />
        ) : (
          <Search className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
        )}
        <button
          type="button"
          onClick={() => setExpanded((current) => !current)}
          aria-expanded={expanded}
          className="flex min-w-0 flex-1 items-center gap-2 text-left"
        >
          <span className="shrink-0 text-muted-foreground">
            {summary.failed ? "Search failed" : "Searched"}
          </span>
          <span className="min-w-0 truncate font-medium">&quot;{summary.query}&quot;</span>
          {!summary.failed && (
            <Badge variant="outline" className="shrink-0 text-muted-foreground">
              {summary.resultCount} {summary.resultCount === 1 ? "result" : "results"}
            </Badge>
          )}
        </button>
        <span className="ml-auto flex min-w-0 shrink items-center gap-1.5 truncate text-xs text-muted-foreground">
          {summary.providerLabel && (
            <SearchProviderIcon
              kind={providerKind(details.provider)}
              provider={accountProvider(details.provider)}
              className="size-3"
            />
          )}
          <span className="truncate">
            {summary.failed
              ? summary.failureReason
              : [
                  summary.providerLabel,
                  summary.fallbackNote ?? formatSearchDuration(summary.durationMs),
                ]
                  .filter(Boolean)
                  .join(" · ")}
          </span>
        </span>
        <Button
          variant="ghost"
          size="icon-sm"
          className="size-6 shrink-0"
          aria-label={expanded ? "Hide results" : "Show results"}
          onClick={() => setExpanded((current) => !current)}
        >
          {expanded ? <ChevronDown className="size-3.5" /> : <ChevronRight className="size-3.5" />}
        </Button>
      </div>

      {expanded && (
        <div className="border-t px-3 py-2">
          {details.answer && (
            <p className="mb-2 text-xs text-muted-foreground">Answer: {details.answer}</p>
          )}
          {shown.length > 0 ? (
            <ul className="space-y-1">
              {shown.map((result) => (
                <li key={result.id ?? result.url} className="grid grid-cols-[9rem_1fr] gap-3">
                  <span className="truncate font-mono text-xs text-muted-foreground">
                    {siteOf(result)}
                  </span>
                  <a
                    href={result.url}
                    target="_blank"
                    rel="noopener noreferrer nofollow"
                    className="truncate hover:underline"
                  >
                    {result.title || result.url}
                  </a>
                </li>
              ))}
            </ul>
          ) : (
            <p className="text-xs text-muted-foreground">
              {summary.failed ? `No provider answered: ${summary.failureReason}` : "No results."}
            </p>
          )}
          <div className="mt-2 flex items-center gap-3 text-xs">
            {more > 0 && <span className="text-muted-foreground">+ {more} more</span>}
            <Button
              variant="link"
              size="sm"
              className="h-auto p-0 text-xs"
              onClick={() => setDrawerOpen(true)}
            >
              Details and raw response
            </Button>
          </div>
        </div>
      )}

      <WebSearchDrawer
        open={drawerOpen}
        onOpenChange={setDrawerOpen}
        details={details}
        durationMs={durationMs}
        event={event}
      />
    </div>
  );
}

/** The search drawer: Details, Raw (per attempt) and the Event payload. */
export function WebSearchDrawer({
  open,
  onOpenChange,
  details,
  durationMs,
  event,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  details: WebSearchDetails;
  durationMs?: number | null;
  event?: unknown;
}) {
  const [tab, setTab] = useState("details");
  const summary = summarizeWebSearch(details, durationMs);
  return (
    <Drawer open={open} onOpenChange={onOpenChange}>
      <DrawerContent className="w-full overflow-y-auto sm:max-w-xl">
        <DrawerHeader>
          <DrawerTitle className="flex items-start gap-2 pr-6 leading-snug">
            <Search className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
            <span>Searched &quot;{details.query}&quot;</span>
          </DrawerTitle>
          <DrawerDescription>{formatSearchDuration(summary.durationMs)} total</DrawerDescription>
        </DrawerHeader>
        <Tabs value={tab} onValueChange={setTab}>
          <TabsList>
            <TabsTrigger value="details">Details</TabsTrigger>
            <TabsTrigger value="raw">Raw</TabsTrigger>
            {event !== undefined && <TabsTrigger value="event">Event</TabsTrigger>}
          </TabsList>
          <TabsContent value="details">
            <SearchDetailsView details={details} durationMs={durationMs} />
          </TabsContent>
          <TabsContent value="raw">
            <SearchRawView details={details} />
          </TabsContent>
          {event !== undefined && (
            <TabsContent value="event">
              <JsonView value={event} />
            </TabsContent>
          )}
        </Tabs>
      </DrawerContent>
    </Drawer>
  );
}

function SectionTitle({ children }: { children: React.ReactNode }) {
  return (
    <h3 className="mt-4 mb-1.5 text-[10px] font-medium tracking-[0.12em] text-muted-foreground uppercase">
      {children}
    </h3>
  );
}

function costLabel(details: WebSearchDetails): string {
  if (!details.ok) return "None";
  if (details.provider === "free") return "Free";
  if (details.provider === "native") return "Billed by the model provider";
  return `Billed on the ${details.provider_label ?? "account"} key`;
}

export function SearchDetailsView({
  details,
  durationMs,
}: {
  details: WebSearchDetails;
  durationMs?: number | null;
}) {
  const summary = summarizeWebSearch(details, durationMs);
  const facts: [string, string][] = [
    ["Answered by", summary.providerLabel ?? "Nobody"],
    ["Setting", settingLabel(details)],
    ["Cost", costLabel(details)],
    ["Results", String(summary.resultCount)],
  ];
  return (
    <div className="text-sm">
      <dl className="grid grid-cols-[7rem_1fr] gap-x-4 gap-y-1.5">
        {facts.map(([label, value]) => (
          <div key={label} className="contents">
            <dt className="text-muted-foreground">{label}</dt>
            <dd>{value}</dd>
          </div>
        ))}
      </dl>

      <SectionTitle>Attempts</SectionTitle>
      <ul className="border" aria-label="Attempts">
        {details.attempts.map((attempt, index) => (
          <AttemptRow key={`${attempt.provider}-${index}`} attempt={attempt} />
        ))}
      </ul>

      <SectionTitle>Results</SectionTitle>
      {details.results.length === 0 ? (
        <p className="text-muted-foreground">No results.</p>
      ) : (
        <ul className="border">
          {details.results.map((result) => (
            <li
              key={result.id ?? result.url}
              className="flex gap-3 border-b px-3 py-2 last:border-b-0"
            >
              <span className="w-14 shrink-0 font-mono text-xs text-muted-foreground">
                {result.id}
              </span>
              <span className="min-w-0 flex-1">
                <a
                  href={result.url}
                  target="_blank"
                  rel="noopener noreferrer nofollow"
                  className="inline-flex max-w-full items-center gap-1 font-medium hover:underline"
                >
                  <span className="truncate">{result.title || result.url}</span>
                  <ExternalLink className="size-3 shrink-0 text-muted-foreground" />
                </a>
                <span className="block text-xs text-muted-foreground">
                  {[siteOf(result), result.published].filter(Boolean).join(" · ")}
                </span>
                {result.snippet && (
                  <span className="mt-0.5 line-clamp-2 block text-xs text-muted-foreground">
                    {result.snippet}
                  </span>
                )}
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function AttemptRow({ attempt }: { attempt: SearchAttempt }) {
  const skipped = !attempt.ok && attempt.duration_ms === 0 && !attempt.exchanges?.length;
  return (
    <li className="flex items-center gap-2 border-b px-3 py-1.5 text-sm last:border-b-0">
      <span
        aria-hidden="true"
        className={cn(
          "size-1.5 shrink-0 rounded-full",
          attempt.ok ? "bg-success" : skipped ? "bg-muted-foreground/50" : "bg-destructive",
        )}
      />
      <span className={cn("min-w-0 flex-1 truncate", skipped && "text-muted-foreground")}>
        {attempt.label}
      </span>
      <span
        className={cn(
          "shrink-0 text-xs",
          attempt.ok || skipped ? "text-muted-foreground" : "text-destructive",
        )}
      >
        {attemptStatus(attempt)}
        {!skipped && ` · ${formatSearchDuration(attempt.duration_ms)}`}
      </span>
    </li>
  );
}

function download(name: string, text: string) {
  const url = URL.createObjectURL(new Blob([text], { type: "text/plain" }));
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  link.click();
  URL.revokeObjectURL(url);
}

export function SearchRawView({ details }: { details: WebSearchDetails }) {
  const withExchanges = details.attempts;
  const answered = withExchanges.findIndex((attempt) => attempt.ok);
  const [selected, setSelected] = useState(answered >= 0 ? answered : withExchanges.length - 1);
  const [showAll, setShowAll] = useState(false);
  const attempt = withExchanges[selected];
  const exchanges = attempt?.exchanges ?? [];
  const bodies = exchanges.map((exchange) => exchange.body ?? exchange.error ?? "");

  if (withExchanges.length === 0) {
    return <p className="text-sm text-muted-foreground">No provider was asked.</p>;
  }

  return (
    <div className="space-y-3 text-sm">
      <div className="flex flex-wrap gap-1.5" role="group" aria-label="Attempt">
        {withExchanges.map((candidate, index) => (
          <button
            key={`${candidate.provider}-${index}`}
            type="button"
            aria-pressed={index === selected}
            onClick={() => {
              setSelected(index);
              setShowAll(false);
            }}
            className={cn(
              "border px-2 py-0.5 text-xs",
              index === selected
                ? "border-foreground bg-foreground text-background"
                : candidate.ok
                  ? "text-muted-foreground hover:text-foreground"
                  : "border-destructive/40 text-destructive",
            )}
          >
            {index + 1} · {candidate.label}
          </button>
        ))}
      </div>
      {exchanges.length === 0 ? (
        <p className="text-muted-foreground">
          {attempt?.message
            ? `Nothing was sent: ${attempt.message}.`
            : "This provider kept no raw response."}
        </p>
      ) : (
        exchanges.map((exchange, index) => {
          const body = bodies[index];
          const size = new TextEncoder().encode(body).length;
          const cut = !showAll && body.length > RAW_PREVIEW_BYTES;
          return (
            <div key={`${exchange.url}-${index}`} className="space-y-2">
              <div>
                <div className="mb-1 text-xs text-muted-foreground">Request</div>
                <pre className="overflow-x-auto bg-muted/60 p-2 font-mono text-xs break-all whitespace-pre-wrap">
                  {exchange.method} {exchange.url}
                </pre>
              </div>
              <div>
                <div className="mb-1 flex items-center gap-2 text-xs text-muted-foreground">
                  <span>
                    Response · {exchange.status ?? "no status"} · {formatBytes(size)}
                    {exchange.truncated && " (cut when stored)"}
                    {cut && ` (showing first ${formatBytes(RAW_PREVIEW_BYTES)})`}
                  </span>
                  <span className="ml-auto flex items-center gap-2">
                    <CopyButton value={body} label="Copy response" />
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      className="size-6"
                      aria-label="Download response"
                      onClick={() =>
                        download(`${attempt?.provider ?? "search"}-response.txt`, body)
                      }
                    >
                      <Download className="size-3.5" />
                    </Button>
                  </span>
                </div>
                <pre className="max-h-[50vh] overflow-auto bg-muted/60 p-2 font-mono text-xs break-all whitespace-pre-wrap">
                  {cut ? `${body.slice(0, RAW_PREVIEW_BYTES)}\n…` : body || "(empty)"}
                </pre>
                {cut && (
                  <Button
                    variant="link"
                    size="sm"
                    className="h-auto p-0 text-xs"
                    onClick={() => setShowAll(true)}
                  >
                    Show all
                  </Button>
                )}
              </div>
            </div>
          );
        })
      )}
    </div>
  );
}
