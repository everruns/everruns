"use client";

import { useEffect, useRef, useState } from "react";
import { getEventData, type Event, type SessionStatus } from "@/lib/api/types";
import { submitToolResults } from "@/lib/api/sessions";
import { useChatMode } from "./chat-mode-context";
import {
  PAGE_PATCH_TOOL,
  PAGE_READ_TOOL,
  assertDraftSize,
  type PageDraftDescriptor,
} from "./page-draft";

type ToolResult = { tool_call_id: string; result?: unknown; error?: string };
const isPageTool = (name: string) => name === PAGE_READ_TOOL || name === PAGE_PATCH_TOOL;

/** Execute only typed runtime requests belonging to a send from this browser.
 * Keep results after transport failures: retrying must never apply a patch twice.
 */
export function usePageDraftTools(
  enabled: boolean,
  sessionId: string,
  events: Event[],
  status?: SessionStatus,
) {
  const mode = useChatMode();
  const owned = useRef(new Set<string>());
  const results = useRef(new Map<string, ToolResult[]>());
  const attempted = useRef(new Map<string, number>());
  const busy = useRef(false);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const [delivery, setDelivery] = useState(0);
  useEffect(() => {
    if (!mode) return;
    if (enabled) {
      const binding = mode.pageAdapter?.describe().binding;
      if (binding) {
        owned.current.add(binding);
        if (owned.current.size > 50) owned.current.delete(owned.current.values().next().value!);
      }
    }
    // Full Chat never claims a page. Previously owned requests still need a result
    // after closing the split view or navigating away from their original form.
    if (!owned.current.size) return;
    if (status !== "waiting_for_tool_results" || busy.current) return;
    const request = events.findLast((event) => event.type === "tool.call_requested");
    const calls = request && getEventData(request, "tool.call_requested")?.tool_calls;
    if (!request || !calls?.some((call) => isPageTool(call.name))) return;
    const input = events.find((event) => {
      const data = getEventData(event, "input.message");
      return data && data.message.id === request.context.input_message_id;
    });
    const page = input && getEventData(input, "input.message")?.message.metadata?.page_context;
    const descriptor =
      page && typeof page === "object" && "draft" in page
        ? (page.draft as PageDraftDescriptor)
        : undefined;
    // THREAT[TM-CLIENT-006]: Another tab/browser owns that turn. It alone may execute its UI changes.
    if (!descriptor || !owned.current.has(descriptor.binding)) return;
    const completed = new Set(
      events.flatMap((event) => {
        const data = getEventData(event, "tool.completed");
        return data ? [data.tool_call_id] : [];
      }),
    );
    if (calls.every((call) => completed.has(call.id))) return;
    // A later authored message abandons this client request.
    const latestInput = events.findLast((event) => event.type === "input.message");
    if (latestInput?.id !== input?.id) return;
    const key = `${sessionId}:${request.id}`;
    if (attempted.current.get(key) === attempt || attempted.current.get(key) === -1) return;
    let batch = results.current.get(key);
    if (!batch) {
      const mixed = calls.some((call) => !isPageTool(call.name));
      batch = calls.map((call): ToolResult => {
        try {
          if (mixed)
            throw new Error(
              "Call page draft tools separately from other client tools. No changes were applied.",
            );
          if (completed.has(call.id)) throw new Error("This tool call has already completed.");
          if (
            !mode.pageAdapter ||
            mode.pageAdapter.describe().binding !== descriptor.binding ||
            call.arguments.binding !== descriptor.binding
          )
            throw new Error(
              "The original page or form is no longer available. Ask for a new message from the current page.",
            );
          const result =
            call.name === PAGE_READ_TOOL
              ? mode.pageAdapter.read(descriptor.binding)
              : mode.pageAdapter.patch(
                  descriptor.binding,
                  String(call.arguments.revision ?? ""),
                  call.arguments.changes,
                );
          const pageView =
            call.name === PAGE_READ_TOOL &&
            mode.pageViewAdapter &&
            mode.pageViewAdapter !== mode.pageAdapter
              ? mode.pageViewAdapter.read(mode.pageViewAdapter.describe().binding).values
              : undefined;
          let output: Record<string, unknown> = {
            ...result,
            ...(pageView ? { page_view: pageView } : {}),
            saved: false,
          };
          if (pageView) {
            try {
              assertDraftSize(output);
            } catch {
              // Preserve the complete draft; optional loaded-data summaries must
              // not make an otherwise readable form exceed its existing limit.
              output = { ...result, page_view_omitted_for_size: true, saved: false };
            }
          }
          assertDraftSize(output);
          return { tool_call_id: call.id, result: output };
        } catch (cause) {
          return {
            tool_call_id: call.id,
            error: cause instanceof Error ? cause.message : "Could not access the page draft.",
          };
        }
      });
      results.current.set(key, batch);
      if (results.current.size > 32) {
        const oldest = results.current.keys().next().value!;
        results.current.delete(oldest);
        attempted.current.delete(oldest);
      }
    }
    busy.current = true;
    attempted.current.set(key, attempt);
    void submitToolResults(sessionId, batch)
      .then(() => {
        attempted.current.set(key, -1);
        setError(null);
      })
      .catch(() =>
        setError(
          "Could not return the page result to Chat. Your draft is kept. Retry to continue.",
        ),
      )
      .finally(() => {
        busy.current = false;
        // Resume may publish the next request before this HTTP reply settles.
        // Recheck it without retrying a failed delivery or reapplying a patch.
        setDelivery((value) => value + 1);
      });
  }, [enabled, sessionId, events, status, mode, attempt, delivery]);
  return {
    error: status === "waiting_for_tool_results" ? error : null,
    retry: () => setAttempt((value) => value + 1),
  };
}
