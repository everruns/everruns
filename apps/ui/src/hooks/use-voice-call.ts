"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { endSessionVoice, type VoiceCallResponse } from "@/lib/api/voice";

// WebRTC voice call from the browser. Audio flows browser <-> provider; the
// server only receives the SDP offer, places the call and drives the
// conversation. This hook owns the mic, the peer connection and remote audio
// playback; callers decide which API places the call (session microphone or
// a voice channel).

export type VoiceCallState = "idle" | "connecting" | "connected";

/** A placed call: the session it talks to and the provider answer. */
export interface PlacedVoiceCall {
  sessionId: string;
  voice: VoiceCallResponse;
}

export interface UseVoiceCallOptions {
  /** Sends the local SDP offer to the API and returns the placed call. */
  placeCall: (sdp: string) => Promise<PlacedVoiceCall>;
  /** Called when starting fails (mic permission, API error, WebRTC error). */
  onError?: (error: unknown) => void;
}

export interface UseVoiceCallResult {
  state: VoiceCallState;
  /** Session of the current or last call, if any. */
  sessionId: string | null;
  start: () => Promise<void>;
  stop: (reason?: string) => Promise<void>;
}

export function useVoiceCall({ placeCall, onError }: UseVoiceCallOptions): UseVoiceCallResult {
  const [state, setState] = useState<VoiceCallState>("idle");
  const [sessionId, setSessionId] = useState<string | null>(null);
  const callRef = useRef<{ sessionId: string; voiceConnectionId: string } | null>(null);
  const peerConnectionRef = useRef<RTCPeerConnection | null>(null);
  const mediaStreamRef = useRef<MediaStream | null>(null);
  const remoteAudioRef = useRef<HTMLAudioElement | null>(null);
  const stateRef = useRef<VoiceCallState>("idle");

  const updateState = useCallback((next: VoiceCallState) => {
    stateRef.current = next;
    setState(next);
  }, []);

  const cleanupClient = useCallback(() => {
    peerConnectionRef.current?.close();
    peerConnectionRef.current = null;
    mediaStreamRef.current?.getTracks().forEach((track) => track.stop());
    mediaStreamRef.current = null;
    if (remoteAudioRef.current) {
      remoteAudioRef.current.srcObject = null;
      remoteAudioRef.current.remove();
      remoteAudioRef.current = null;
    }
  }, []);

  const stop = useCallback(
    async (reason = "client_ended") => {
      const call = callRef.current;
      callRef.current = null;
      cleanupClient();
      updateState("idle");
      if (!call) return;
      try {
        await endSessionVoice(call.sessionId, call.voiceConnectionId, reason);
      } catch (error) {
        console.error("Failed to end voice session:", error);
      }
    },
    [cleanupClient, updateState],
  );

  // End the call when the component using the hook unmounts.
  useEffect(() => {
    return () => {
      void stop("unmounted");
    };
  }, [stop]);

  const start = useCallback(async () => {
    if (stateRef.current !== "idle") return;
    updateState("connecting");
    try {
      const mediaStream = await navigator.mediaDevices.getUserMedia({ audio: true });
      const peerConnection = new RTCPeerConnection();
      peerConnectionRef.current = peerConnection;
      mediaStreamRef.current = mediaStream;
      mediaStream.getTracks().forEach((track) => peerConnection.addTrack(track, mediaStream));
      const remoteAudio = document.createElement("audio");
      remoteAudio.autoplay = true;
      remoteAudio.setAttribute("playsinline", "true");
      remoteAudioRef.current = remoteAudio;
      peerConnection.ontrack = (event) => {
        remoteAudio.srcObject = event.streams[0];
      };
      const offer = await peerConnection.createOffer();
      await peerConnection.setLocalDescription(offer);
      if (!offer.sdp) {
        throw new Error("Missing local voice offer.");
      }
      const placed = await placeCall(offer.sdp);
      // Record the call before the answer so a failed handshake still ends it.
      callRef.current = {
        sessionId: placed.sessionId,
        voiceConnectionId: placed.voice.voice_connection_id,
      };
      setSessionId(placed.sessionId);
      await peerConnection.setRemoteDescription({
        type: "answer",
        sdp: placed.voice.answer_sdp,
      });
      document.body.appendChild(remoteAudio);
      updateState("connected");
    } catch (error) {
      await stop("client_error");
      onError?.(error);
    }
  }, [onError, placeCall, stop, updateState]);

  return { state, sessionId, start, stop };
}
