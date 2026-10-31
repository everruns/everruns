"use client";

// Decision: saved changes surface as a small bar floating at the bottom of the
// page, not a panel under it. A save is routine; review and undo are one click
// away without taking space from the form. The bar renders nothing until a
// save produces a receipt, and leaves a short "Undid" confirmation that fades.

import { useEffect, useRef, useState } from "react";
import { AlertTriangle, Check, Undo2, X } from "lucide-react";
import { useUiTranslations, useLocale } from "@/providers/locale-provider";
import { formatSavedChangeCount, formatUndoneChangeCount } from "@/lib/i18n";
import { isSecretField } from "@/lib/api/change-history";
import type { FieldDiff } from "@/lib/api/types";
import type { PageResource } from "./page-resource-history";
import type { SavedPageChange } from "./page-change-review-controller";

const UNDONE_NOTICE_MS = 5_000;
const INLINE_LIMIT = 48;

export interface SavedChangesBarProps {
  receipt: SavedPageChange;
  dirty: boolean;
  onUndo: (field?: string) => void;
  onDismiss: () => void;
  onHistory: (resource: PageResource) => void;
  fieldLabels?: Record<string, string>;
  /** Start with Review open (previews). */
  initiallyOpen?: boolean;
}

function valueText(value: unknown, ui: (key: string) => string): string {
  if (value === undefined) return ui("Not set");
  if (value === null) return ui("None");
  return typeof value === "string" ? value || ui("Empty") : JSON.stringify(value, null, 2);
}

function inline(text: string) {
  return !text.includes("\n") && text.length <= INLINE_LIMIT;
}

function DiffRow({
  diff,
  label,
  canUndo,
  onUndo,
}: {
  diff: FieldDiff;
  label: string;
  canUndo: boolean;
  onUndo: () => void;
}) {
  const ui = useUiTranslations();
  const [open, setOpen] = useState(false);
  const before = valueText(diff.from, ui),
    after = valueText(diff.to, ui);
  const short = inline(before) && inline(after);
  return (
    <li
      className="grid grid-cols-[minmax(6rem,9rem)_minmax(0,1fr)_auto] items-start gap-3 px-3 py-2"
      aria-label={ui("Change to {value0}", { value0: label })}
    >
      <span className="truncate text-muted-foreground first-letter:uppercase">{label}</span>
      {short ? (
        <span className="min-w-0 break-words">
          <del className="text-destructive/90">{before}</del>
          <span className="mx-1.5 text-muted-foreground" aria-hidden>
            →
          </span>
          <ins className="text-success no-underline">{after}</ins>
        </span>
      ) : (
        <span className="min-w-0">
          <button
            type="button"
            className="text-left underline underline-offset-2"
            aria-expanded={open}
            onClick={() => setOpen(!open)}
          >
            {open ? ui("Hide values") : ui("Show values")}
          </button>
          {open && (
            <span className="mt-2 grid gap-2 sm:grid-cols-2">
              <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-words border-l-2 border-destructive/50 bg-muted/40 p-2 text-xs">
                {before}
              </pre>
              <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-words border-l-2 border-success bg-muted/40 p-2 text-xs">
                {after}
              </pre>
            </span>
          )}
        </span>
      )}
      <button
        type="button"
        className="text-muted-foreground underline underline-offset-2 hover:text-foreground disabled:opacity-50"
        disabled={!canUndo}
        aria-label={`${ui("Undo")} ${label}`}
        onClick={onUndo}
      >
        {ui("Undo")}
      </button>
    </li>
  );
}

const barClass =
  "pointer-events-auto flex max-w-full items-center gap-3 border bg-card py-1.5 pl-3 pr-1.5 text-sm shadow-lg";
const linkClass =
  "whitespace-nowrap underline underline-offset-2 disabled:no-underline disabled:opacity-50";

export function SavedChangesBar({
  receipt,
  dirty,
  onUndo,
  onDismiss,
  onHistory,
  fieldLabels,
  initiallyOpen = false,
}: SavedChangesBarProps) {
  const ui = useUiTranslations();
  const { locale } = useLocale();
  const [open, setOpen] = useState(initiallyOpen);
  const fields = receipt.fields.filter((field) => !isSecretField(field.field));
  const canUndo = !receipt.pending && !receipt.stale && !dirty;
  const history = receipt.resource.kind !== "model_selection";
  const label = (field: string) => fieldLabels?.[field] ?? field.replaceAll("_", " ");
  const count = fields.length;
  if (!count && !receipt.error) return null;
  return (
    <div className="flex max-w-full flex-col items-center gap-2">
      {open && !receipt.stale && count > 0 && (
        <section
          aria-label={ui("Saved page changes")}
          className="pointer-events-auto w-[34rem] max-w-full border bg-card text-sm shadow-lg"
        >
          <header className="flex items-center justify-between gap-3 border-b px-3 py-2">
            <strong className="truncate font-medium">
              {formatSavedChangeCount(locale, count)} · {receipt.resource.label}
            </strong>
            {history && (
              <button
                type="button"
                className={`${linkClass} text-muted-foreground`}
                onClick={() => onHistory(receipt.resource)}
              >
                {ui("History")}
              </button>
            )}
          </header>
          <ul className="max-h-[40vh] divide-y overflow-auto">
            {fields.map((diff) => (
              <DiffRow
                key={diff.field}
                diff={diff}
                label={label(diff.field)}
                canUndo={canUndo}
                onUndo={() => onUndo(diff.field)}
              />
            ))}
          </ul>
          {dirty && (
            <p className="border-t px-3 py-2 text-xs text-muted-foreground">
              {ui("Your draft is kept. Save or discard it before undoing saved changes.")}
            </p>
          )}
        </section>
      )}
      <div role="status" className={barClass}>
        {receipt.stale ? (
          <>
            <AlertTriangle className="size-3.5 shrink-0 text-accent" aria-hidden />
            <span className="truncate">{ui("Changed again since your save")}</span>
            <span className="h-4 w-px shrink-0 bg-border" aria-hidden />
            {history && (
              <button
                type="button"
                className={linkClass}
                onClick={() => onHistory(receipt.resource)}
              >
                {ui("History")}
              </button>
            )}
          </>
        ) : (
          <>
            <Check className="size-3.5 shrink-0 text-success" aria-hidden />
            <span className="truncate">
              {receipt.pending ? ui("Undoing…") : formatSavedChangeCount(locale, count)}
            </span>
            <span className="h-4 w-px shrink-0 bg-border" aria-hidden />
            <button
              type="button"
              className={linkClass}
              aria-expanded={open}
              onClick={() => setOpen(!open)}
            >
              {open ? ui("Hide") : ui("Review")}
            </button>
            <button
              type="button"
              className={linkClass}
              disabled={!canUndo}
              title={
                dirty
                  ? ui("Your draft is kept. Save or discard it before undoing saved changes.")
                  : undefined
              }
              onClick={() => onUndo()}
            >
              {open ? ui("Undo all") : ui("Undo")}
            </button>
          </>
        )}
        {receipt.error && (
          <span role="alert" className="truncate text-destructive">
            {receipt.error}
          </span>
        )}
        <button
          type="button"
          className="p-1 text-muted-foreground hover:text-foreground"
          aria-label={ui("Dismiss")}
          onClick={onDismiss}
        >
          <X className="size-3.5" />
        </button>
      </div>
    </div>
  );
}

/** Confirms a completed Undo all, then removes itself. */
export function UndoneBar({
  count,
  resource,
  onHistory,
  onDone,
}: {
  count: number;
  resource: PageResource;
  onHistory: (resource: PageResource) => void;
  onDone: () => void;
}) {
  const ui = useUiTranslations();
  const { locale } = useLocale();
  const done = useRef(onDone);
  useEffect(() => {
    done.current = onDone;
  });
  useEffect(() => {
    const timer = setTimeout(() => done.current(), UNDONE_NOTICE_MS);
    return () => clearTimeout(timer);
  }, []);
  return (
    <div role="status" className={barClass}>
      <Undo2 className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
      <span className="truncate">{formatUndoneChangeCount(locale, count)}</span>
      {resource.kind !== "model_selection" && (
        <>
          <span className="h-4 w-px shrink-0 bg-border" aria-hidden />
          <button
            type="button"
            className={`${linkClass} text-muted-foreground`}
            onClick={() => onHistory(resource)}
          >
            {ui("History")}
          </button>
        </>
      )}
      <button
        type="button"
        className="p-1 text-muted-foreground hover:text-foreground"
        aria-label={ui("Dismiss")}
        onClick={onDone}
      >
        <X className="size-3.5" />
      </button>
    </div>
  );
}
