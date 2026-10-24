"use client";

import { createContext, useContext, type RefObject } from "react";
import type { PageDraftAdapter, PageDraftDescriptor } from "./page-draft";

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
  actions: PageAction[];
  recordAction: (label: string) => void;
}

// Optional: pages and ordinary chat surfaces keep working outside the new shell.
export const ChatModeContext = createContext<ChatModeState | null>(null);
export const useChatMode = () => useContext(ChatModeContext);
