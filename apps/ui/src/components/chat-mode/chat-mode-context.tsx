"use client";

import { createContext, useContext, type RefObject } from "react";
import type { PageDraftAdapter, PageDraftDescriptor } from "./page-draft";
import { PageViewRegistryContext } from "./page-view-state";

export interface ChatPageContext {
  route: string;
  label: string;
  entity_type?: string;
  entity_id?: string;
  editing?: boolean;
  form_name?: string;
  draft?: PageDraftDescriptor;
}

export interface PageAction {
  id: string;
  label: string;
  page: ChatPageContext;
}

export interface ChatModeState {
  pageContainer: RefObject<HTMLElement | null>;
  pageContainerElement?: HTMLElement | null;
  open: boolean;
  setOpen: (open: boolean) => void;
  page: ChatPageContext;
  registerPage: (
    page: ChatPageContext,
    adapter?: PageDraftAdapter,
    priority?: number,
  ) => () => void;
  pageAdapter?: PageDraftAdapter;
  pageViewAdapter?: PageDraftAdapter;
  threadId?: string;
  /** Show this chat (Main chat when omitted) beside the current page. */
  openBeside?: (chatId?: string) => void;
  /** Hide the page and show the conversation full screen. */
  showFullScreen?: () => void;
  /** Return from full-screen Chat to the last page, with Chat beside it. */
  showBesidePage?: () => void;
  /** Show a thread in the page pane, beside the conversation. */
  openThreadBeside?: (threadId: string) => void;
  actions: PageAction[];
  recordAction: (label: string) => void;
}

// Optional: pages and ordinary chat surfaces keep working outside the new shell.
export const ChatModeContext = createContext<ChatModeState | null>(null);
export const useChatMode = () => useContext(ChatModeContext);

// Only the right-page subtree may register its forms or scope an overlay there.
// Chat, sidebar and global overlays retain their own interaction behavior.
export function usePageChatMode() {
  const mode = useChatMode();
  const page = useContext(PageViewRegistryContext);
  return page ? mode : null;
}
