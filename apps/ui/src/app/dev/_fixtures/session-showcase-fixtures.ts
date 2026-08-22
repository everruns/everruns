"use client";

import type { Agent, ModelWithProvider, Session, SessionStatus, TokenUsage } from "@/lib/api/types";
import type { SessionNavKey } from "@/components/session/session-header";

const agent: Agent = {
  id: "agent_session_showcase",
  name: "platform-chat",
  harness_id: "harness_test",
  display_name: "Platform Chat",
  description: "Showcase agent for session UI states.",
  system_prompt: "You are a precise coding assistant.",
  default_model_id: "model-kimi-k2.5",
  tags: [],
  capabilities: [],
  initial_files: [],
  status: "active",
  created_at: "2026-04-22T14:00:00Z",
  updated_at: "2026-04-22T14:00:00Z",
  archived_at: null,
  deleted_at: null,
};

const model: ModelWithProvider = {
  id: "model-kimi-k2.5",
  provider_id: "provider-openrouter",
  model_id: "openrouter/kimi-k2.5",
  display_name: "Kimi K2.5",
  capabilities: ["chat", "tools"],
  enabled: true,
  is_favorite: true,
  healthy: true,
  created_at: "2026-04-22T14:00:00Z",
  updated_at: "2026-04-22T14:00:00Z",
  provider_name: "OpenRouter",
  provider_type: "openai",
};

function createSession({
  id,
  title,
  status,
  features,
  activeScheduleCount = 0,
  taskCount,
  eventCount,
  fileCount,
  usage,
}: {
  id: string;
  title: string;
  status: SessionStatus;
  features: string[];
  activeScheduleCount?: number;
  taskCount?: number;
  eventCount?: number;
  fileCount?: number;
  usage?: TokenUsage;
}): Session {
  return {
    id,
    organization_id: "org_session_showcase",
    harness_id: "harness_generic",
    agent_id: agent.id,
    owner_principal_id: "user_session_showcase",
    title,
    preview: "Make the tool transcript read like progress instead of raw logs.",
    output_preview: "The preview now uses the same session chrome as the main view.",
    tags: ["showcase"],
    model_id: model.id,
    status,
    created_at: "2026-04-22T14:20:00Z",
    updated_at: "2026-04-22T14:25:00Z",
    started_at: "2026-04-22T14:20:00Z",
    finished_at: null,
    active_schedule_count: activeScheduleCount,
    task_count: taskCount,
    event_count: eventCount,
    file_count: fileCount,
    features,
    usage,
  };
}

export const sessionShowcaseAgent = agent;
export const sessionShowcaseModel = model;

export const sessionUsageSamples = {
  light: {
    input_tokens: 128,
    output_tokens: 45,
    cache_read_tokens: 0,
    cache_creation_tokens: 0,
  } satisfies TokenUsage,
  medium: {
    input_tokens: 1842,
    output_tokens: 326,
    cache_read_tokens: 512,
    cache_creation_tokens: 0,
  } satisfies TokenUsage,
  heavy: {
    input_tokens: 15234,
    output_tokens: 8721,
    cache_read_tokens: 5000,
    cache_creation_tokens: 1200,
  } satisfies TokenUsage,
};

export const sessionHeaderScenarios: Array<{
  name: string;
  session: Session;
  activeTab: SessionNavKey;
  effectiveStatus: SessionStatus;
  liveUsage?: TokenUsage;
  secondaryMetaText: string;
}> = [
  {
    name: "Active recording",
    session: createSession({
      id: "session_showcase_active",
      title: "Tool transcript polish",
      status: "active",
      features: ["file_system", "leased_resources", "schedules"],
      activeScheduleCount: 2,
      taskCount: 3,
      eventCount: 428,
      fileCount: 6,
    }),
    activeTab: "timeline",
    effectiveStatus: "active",
    liveUsage: sessionUsageSamples.medium,
    secondaryMetaText: "Started 4/22/2026, 9:20:00 AM",
  },
  {
    name: "Workspace view",
    session: createSession({
      id: "session_showcase_workspace",
      title: "Sandbox file inspection",
      status: "idle",
      features: ["file_system", "key_value"],
      eventCount: 12,
      fileCount: 4,
      usage: sessionUsageSamples.light,
    }),
    activeTab: "files",
    effectiveStatus: "idle",
    liveUsage: sessionUsageSamples.light,
    secondaryMetaText: "Workspace: /workspace",
  },
  {
    name: "Waiting on client tools",
    session: createSession({
      id: "session_showcase_waiting",
      title: "Annotate the screenshot",
      status: "waiting_for_tool_results",
      features: ["file_system", "leased_resources", "schedules"],
      activeScheduleCount: 1,
      usage: sessionUsageSamples.heavy,
    }),
    activeTab: "timeline",
    effectiveStatus: "waiting_for_tool_results",
    liveUsage: sessionUsageSamples.heavy,
    secondaryMetaText: "Started 4/22/2026, 9:20:00 AM",
  },
];

export const sessionCardScenarios = [
  {
    name: "Running",
    session: createSession({
      id: "session_card_running",
      title: "Daytona sandbox investigation",
      status: "active",
      features: ["file_system", "leased_resources"],
      activeScheduleCount: 1,
      usage: sessionUsageSamples.medium,
    }),
    summary: "Run the sandbox flow and show the exact tool transcript.",
  },
  {
    name: "Idle",
    session: createSession({
      id: "session_card_idle",
      title: "Chat UI review",
      status: "idle",
      features: ["file_system"],
      usage: sessionUsageSamples.light,
    }),
    summary: "The chat UI preview now points at the production transcript stack.",
  },
  {
    name: "New",
    session: createSession({
      id: "session_card_new",
      title: "Session components",
      status: "started",
      features: ["file_system", "schedules"],
    }),
    summary: "Showcase headers, badges, and session cards in one place.",
  },
];
