"use client";
import type { ReactNode } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { Button } from "./button";
import { useUiTranslations } from "@/providers/locale-provider";
import { cn } from "@/lib/utils";

export function Pagination({
  page,
  totalPages,
  onPageChange,
  summary,
  className,
}: {
  page: number;
  totalPages: number;
  onPageChange: (page: number) => void;
  summary?: ReactNode;
  className?: string;
}) {
  const ui = useUiTranslations();
  if (totalPages <= 1 && !summary) return null;
  return (
    <div
      className={cn(
        "flex flex-wrap items-center justify-between gap-3 text-sm text-muted-foreground",
        className,
      )}
    >
      {summary && <span>{summary}</span>}
      {totalPages > 1 && (
        <nav aria-label={ui("Pagination")} className="flex flex-wrap items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            disabled={page <= 0}
            onClick={() => onPageChange(Math.max(0, page - 1))}
          >
            <ChevronLeft className="size-4" />
            {ui("Previous")}
          </Button>
          <span aria-live="polite">
            {ui("Page {value0} of {value1}", { value0: page + 1, value1: totalPages })}
          </span>
          <Button
            variant="outline"
            size="sm"
            disabled={page >= totalPages - 1}
            onClick={() => onPageChange(Math.min(totalPages - 1, page + 1))}
          >
            {ui("Next")}
            <ChevronRight className="size-4" />
          </Button>
        </nav>
      )}
    </div>
  );
}
