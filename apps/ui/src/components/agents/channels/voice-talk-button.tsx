"use client";

// "Talk to this channel": a test call to a voice channel from the browser. The
// call starts a new session on the channel's agent; the link opens it so the
// transcript and the agent's work can be inspected.

import { useCallback, useState } from "react";
import Link from "next/link";
import { Mic, PhoneOff } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useVoiceCall } from "@/hooks/use-voice-call";
import { startChannelVoice } from "@/lib/api/voice";

function describeError(error: unknown): string {
  if (error && typeof error === "object" && "name" in error) {
    const name = String(error.name).toLowerCase();
    if (name === "notallowederror" || name === "securityerror") {
      return "Microphone access is blocked. Allow it in your browser settings and try again.";
    }
  }
  return error instanceof Error ? error.message : "Could not start the call.";
}

export function VoiceTalkButton({
  agentId,
  channelId,
  disabled = false,
}: {
  agentId: string;
  channelId: string;
  disabled?: boolean;
}) {
  const [error, setError] = useState<string | null>(null);
  const placeCall = useCallback(
    async (sdp: string) => {
      const { session, voice } = await startChannelVoice(agentId, channelId, { sdp });
      return { sessionId: session.id, voice };
    },
    [agentId, channelId],
  );
  const onError = useCallback((caught: unknown) => setError(describeError(caught)), []);
  const call = useVoiceCall({ placeCall, onError });
  const microphoneAvailable =
    typeof navigator !== "undefined" &&
    !!navigator.mediaDevices?.getUserMedia &&
    typeof RTCPeerConnection !== "undefined";
  const ended = call.state === "idle" && !!call.sessionId;

  const start = () => {
    setError(null);
    void call.start();
  };

  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-2">
        {call.state === "idle" ? (
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={start}
            disabled={disabled || !microphoneAvailable}
          >
            <Mic className="size-4" />
            {ended ? "Talk again" : "Talk to this channel"}
          </Button>
        ) : (
          <Button type="button" variant="outline" size="sm" onClick={() => void call.stop()}>
            <PhoneOff className="size-4" />
            Hang up
          </Button>
        )}
        <span aria-live="polite" className="flex items-center gap-2">
          {call.state === "connecting" && <Badge variant="secondary">Connecting…</Badge>}
          {call.state === "connected" && <Badge>Live</Badge>}
          {ended && <Badge variant="secondary">Ended</Badge>}
        </span>
        {call.sessionId && (
          <Link className="text-sm underline" href={`/sessions/${call.sessionId}`}>
            Open session
          </Link>
        )}
      </div>
      {!microphoneAvailable && (
        <p className="text-xs text-muted-foreground">This browser cannot place voice calls.</p>
      )}
      {error && (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      )}
    </div>
  );
}
