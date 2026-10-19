"use client";

// Voice channel form fields and their state <-> config mapping. Kept out of
// channel-form.tsx, which is near the file-size limit. The config mirrors
// `VoiceChannelConfig` in crates/contracts/src/voice.rs; limits match its
// `validate()`.

import { useId, useState } from "react";
import { ChevronDown } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import type {
  VoiceChannelConfig,
  VoiceInterruption,
  VoiceTurnDetection,
} from "@/lib/api/channel-types";
import { cn } from "@/lib/utils";

export const DEFAULT_VOICE_MODEL = "gpt-realtime-2";
export const DEFAULT_VOICE = "marin";
export const DEFAULT_VOICE_FILLER = "One moment.";
export const DEFAULT_VOICE_FILLER_AFTER_MS = 1500;
const MAX_GREETING = 500;
const MAX_SPEAKING_STYLE = 4000;
const MAX_FILLER_AFTER_MS = 60_000;

/** Common OpenAI realtime voices. A saved voice outside this list is still offered. */
export const OPENAI_VOICES = [
  "marin",
  "cedar",
  "alloy",
  "ash",
  "ballad",
  "coral",
  "echo",
  "sage",
  "shimmer",
  "verse",
];

export interface VoiceFormState {
  model: string;
  voice: string;
  language: string;
  greeting: string;
  turnDetection: VoiceTurnDetection;
  interruption: VoiceInterruption;
  filler: string;
  /** Kept as text so the input can be edited freely; parsed on save. */
  fillerAfterMs: string;
  speakingStyle: string;
}

export function voiceFormStateFromConfig(config?: VoiceChannelConfig): VoiceFormState {
  return {
    model: config?.model || DEFAULT_VOICE_MODEL,
    voice: config?.voice || DEFAULT_VOICE,
    language: config?.language ?? "",
    greeting: config?.greeting ?? "",
    turnDetection: config?.turn_detection ?? "server_vad",
    interruption: config?.interruption ?? "steer",
    filler: config?.filler ?? DEFAULT_VOICE_FILLER,
    fillerAfterMs: String(config?.filler_after_ms ?? DEFAULT_VOICE_FILLER_AFTER_MS),
    speakingStyle: config?.speaking_style ?? "",
  };
}

function parseFillerAfterMs(value: string): number | null {
  const trimmed = value.trim();
  if (!/^\d+$/.test(trimmed)) return null;
  const parsed = Number.parseInt(trimmed, 10);
  return parsed <= MAX_FILLER_AFTER_MS ? parsed : null;
}

export function buildVoiceChannelConfig(state: VoiceFormState): VoiceChannelConfig {
  const optional = (key: "language" | "greeting" | "speaking_style", value: string) =>
    value.trim() ? { [key]: value.trim() } : {};
  return {
    mode: "delegated",
    model: state.model.trim() || DEFAULT_VOICE_MODEL,
    voice: state.voice.trim() || DEFAULT_VOICE,
    ...optional("language", state.language),
    ...optional("greeting", state.greeting),
    turn_detection: state.turnDetection,
    interruption: state.interruption,
    filler_after_ms: parseFillerAfterMs(state.fillerAfterMs) ?? DEFAULT_VOICE_FILLER_AFTER_MS,
    filler: state.filler.trim() || DEFAULT_VOICE_FILLER,
    ...optional("speaking_style", state.speakingStyle),
  };
}

export function isVoiceFormValid(state: VoiceFormState): boolean {
  return (
    parseFillerAfterMs(state.fillerAfterMs) !== null &&
    state.greeting.trim().length <= MAX_GREETING &&
    state.speakingStyle.trim().length <= MAX_SPEAKING_STYLE
  );
}

export function VoiceFields({
  value,
  onChange,
}: {
  value: VoiceFormState;
  onChange: (value: VoiceFormState) => void;
}) {
  const id = useId();
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const set = <K extends keyof VoiceFormState>(key: K, next: VoiceFormState[K]) =>
    onChange({ ...value, [key]: next });
  const voices = OPENAI_VOICES.includes(value.voice)
    ? OPENAI_VOICES
    : [value.voice, ...OPENAI_VOICES];
  const fillerDelayValid = parseFillerAfterMs(value.fillerAfterMs) !== null;

  return (
    <div className="space-y-4">
      <p className="text-xs text-muted-foreground">
        A speech model listens and speaks; this agent, with its own model and tools, writes every
        answer. Business rules belong in the agent, not here.
      </p>
      <div className="grid gap-4 md:grid-cols-2">
        <div className="space-y-2">
          <Label htmlFor={`${id}_voice`}>Voice</Label>
          <Select value={value.voice} onValueChange={(next) => set("voice", String(next ?? ""))}>
            <SelectTrigger id={`${id}_voice`}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {voices.map((voice) => (
                <SelectItem key={voice} value={voice}>
                  {voice}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-2">
          <Label htmlFor={`${id}_language`}>Language hint</Label>
          <Input
            id={`${id}_language`}
            value={value.language}
            onChange={(event) => set("language", event.target.value)}
            placeholder="Auto-detect (e.g. en, de, uk)"
          />
        </div>
      </div>

      <div className="space-y-2">
        <Label htmlFor={`${id}_greeting`}>Greeting</Label>
        <Textarea
          id={`${id}_greeting`}
          value={value.greeting}
          onChange={(event) => set("greeting", event.target.value)}
          maxLength={MAX_GREETING}
          rows={2}
          placeholder="Hi, you're talking to an AI assistant for Acme. How can I help?"
        />
        <p className="text-xs text-muted-foreground">
          Spoken when the call connects. Tell callers they are talking to an AI.
        </p>
      </div>

      <div className="grid gap-4 md:grid-cols-2">
        <div className="space-y-2">
          <Label htmlFor={`${id}_interruption`}>When the caller interrupts</Label>
          <Select
            value={value.interruption}
            onValueChange={(next) => set("interruption", next as VoiceInterruption)}
          >
            <SelectTrigger id={`${id}_interruption`}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="steer">Keep working, take it into account</SelectItem>
              <SelectItem value="cancel">Stop and start over</SelectItem>
            </SelectContent>
          </Select>
          <p className="text-xs text-muted-foreground">
            Speech always stops. Keep working lets the caller&apos;s next words steer the answer in
            progress; stop and start over cancels it.
          </p>
        </div>
        <div className="space-y-2">
          <Label htmlFor={`${id}_turn_detection`}>End of turn</Label>
          <Select
            value={value.turnDetection}
            onValueChange={(next) => set("turnDetection", next as VoiceTurnDetection)}
          >
            <SelectTrigger id={`${id}_turn_detection`}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="server_vad">Pause-based</SelectItem>
              <SelectItem value="semantic_vad">Waits for a finished thought</SelectItem>
            </SelectContent>
          </Select>
          <p className="text-xs text-muted-foreground">
            Pause-based answers after a short silence. Waits for a finished thought rides through
            mid-sentence pauses.
          </p>
        </div>
      </div>

      <div className="grid gap-4 md:grid-cols-2">
        <div className="space-y-2">
          <Label htmlFor={`${id}_filler`}>Filler line</Label>
          <Input
            id={`${id}_filler`}
            value={value.filler}
            onChange={(event) => set("filler", event.target.value)}
            placeholder={DEFAULT_VOICE_FILLER}
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor={`${id}_filler_after`}>Filler delay (ms)</Label>
          <Input
            id={`${id}_filler_after`}
            value={value.fillerAfterMs}
            onChange={(event) => set("fillerAfterMs", event.target.value)}
            inputMode="numeric"
            aria-invalid={!fillerDelayValid}
            placeholder={String(DEFAULT_VOICE_FILLER_AFTER_MS)}
          />
          <p className="text-xs text-muted-foreground">
            Spoken while the agent works and has said nothing yet. 0 turns fillers off; max 60000.
          </p>
        </div>
      </div>

      <div className="space-y-2">
        <Label htmlFor={`${id}_speaking_style`}>Speaking style</Label>
        <Textarea
          id={`${id}_speaking_style`}
          value={value.speakingStyle}
          onChange={(event) => set("speakingStyle", event.target.value)}
          maxLength={MAX_SPEAKING_STYLE}
          rows={3}
          placeholder="Warm and brief. Spell out numbers digit by digit."
        />
      </div>

      <Collapsible open={advancedOpen} onOpenChange={setAdvancedOpen} className="border-t pt-4">
        <CollapsibleTrigger asChild>
          <button
            type="button"
            className="flex w-full items-center gap-2 text-left text-sm font-medium outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <ChevronDown
              className={cn(
                "size-4 shrink-0 text-muted-foreground transition-transform",
                !advancedOpen && "-rotate-90",
              )}
            />
            <span>Advanced</span>
          </button>
        </CollapsibleTrigger>
        <CollapsibleContent className="space-y-2 pt-4">
          <Label htmlFor={`${id}_model`}>Speech model</Label>
          <Input
            id={`${id}_model`}
            value={value.model}
            onChange={(event) => set("model", event.target.value)}
            className="font-mono"
            placeholder={DEFAULT_VOICE_MODEL}
          />
        </CollapsibleContent>
      </Collapsible>
    </div>
  );
}
