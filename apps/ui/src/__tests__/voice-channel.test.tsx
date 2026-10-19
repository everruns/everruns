import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import {
  buildChannelConfig,
  ChannelTypePicker,
  getDefaultChannelFormState,
  isChannelFormValid,
} from "@/components/agents/channels/channel-form";
import { VoiceTalkButton } from "@/components/agents/channels/voice-talk-button";
import type { AgentChannel } from "@/lib/api/types";

const mockUseFeatureFlag = jest.fn();
const mockStartChannelVoice = jest.fn();
const mockEndSessionVoice = jest.fn();

jest.mock("@/providers/feature-flags-provider", () => ({
  useFeatureFlag: (...args: unknown[]) => mockUseFeatureFlag(...args),
}));

jest.mock("@/lib/api/voice", () => ({
  startChannelVoice: (...args: unknown[]) => mockStartChannelVoice(...args),
  endSessionVoice: (...args: unknown[]) => mockEndSessionVoice(...args),
}));

describe("voice channel kind", () => {
  beforeEach(() => mockUseFeatureFlag.mockReset());

  it("is offered only when the voice feature flag is on", () => {
    mockUseFeatureFlag.mockImplementation((flag: string) => flag !== "voice");
    const { rerender } = render(<ChannelTypePicker value="webhook" onChange={jest.fn()} />);
    expect(screen.queryByText("Voice")).not.toBeInTheDocument();

    mockUseFeatureFlag.mockImplementation(() => true);
    rerender(<ChannelTypePicker value="webhook" onChange={jest.fn()} />);
    expect(screen.getByText("Voice")).toBeInTheDocument();
  });

  it("builds the default voice config", () => {
    const state = getDefaultChannelFormState("voice");
    expect(isChannelFormValid(state)).toBe(true);
    expect(buildChannelConfig(state)).toEqual({
      mode: "delegated",
      model: "gpt-realtime-2",
      voice: "marin",
      turn_detection: "server_vad",
      interruption: "steer",
      filler_after_ms: 1500,
      filler: "One moment.",
    });
  });

  it("round-trips a saved voice channel and trims optional fields", () => {
    const channel = {
      channel_type: "voice",
      enabled: true,
      channel_config: {
        mode: "delegated",
        model: "gpt-realtime-2",
        voice: "cedar",
        language: "uk",
        greeting: "Hi, you are talking to an AI assistant.",
        turn_detection: "semantic_vad",
        interruption: "cancel",
        filler_after_ms: 0,
        filler: "Hold on.",
        speaking_style: "Brief.",
      },
    } as unknown as AgentChannel;
    const state = getDefaultChannelFormState("voice", channel);
    expect(state.kind).toBe("voice");
    expect(buildChannelConfig(state)).toEqual(channel.channel_config);

    const cleared = {
      ...state,
      voice: { ...state.voice, language: "  ", speakingStyle: "" },
    };
    const config = buildChannelConfig(cleared);
    expect(config).not.toHaveProperty("language");
    expect(config).not.toHaveProperty("speaking_style");
  });

  it("rejects an out-of-range filler delay", () => {
    const state = getDefaultChannelFormState("voice");
    expect(
      isChannelFormValid({
        ...state,
        voice: { ...state.voice, fillerAfterMs: "60001" },
      }),
    ).toBe(false);
    expect(
      isChannelFormValid({
        ...state,
        voice: { ...state.voice, fillerAfterMs: "abc" },
      }),
    ).toBe(false);
  });
});

describe("VoiceTalkButton", () => {
  const stopTrack = jest.fn();
  const close = jest.fn();
  const setRemoteDescription = jest.fn();

  beforeEach(() => {
    jest.clearAllMocks();
    class MockRTCPeerConnection {
      ontrack: ((event: { streams: MediaStream[] }) => void) | null = null;
      addTrack = jest.fn();
      close = close;
      createOffer = jest.fn().mockResolvedValue({ type: "offer", sdp: "local-sdp" });
      setLocalDescription = jest.fn();
      setRemoteDescription = setRemoteDescription;
    }
    Object.defineProperty(navigator, "mediaDevices", {
      configurable: true,
      value: {
        getUserMedia: jest.fn().mockResolvedValue({ getTracks: () => [{ stop: stopTrack }] }),
      },
    });
    Object.defineProperty(globalThis, "RTCPeerConnection", {
      configurable: true,
      value: MockRTCPeerConnection,
    });
  });

  it("calls the channel, goes live, links the session and hangs up", async () => {
    mockStartChannelVoice.mockResolvedValue({
      session: { id: "session-9" },
      voice: { voice_connection_id: "voice-9", answer_sdp: "remote-sdp" },
    });
    mockEndSessionVoice.mockResolvedValue(undefined);

    render(<VoiceTalkButton agentId="agent-1" channelId="appchan_1" />);
    fireEvent.click(screen.getByRole("button", { name: /talk to this channel/i }));

    expect(await screen.findByText("Live")).toBeInTheDocument();
    expect(mockStartChannelVoice).toHaveBeenCalledWith("agent-1", "appchan_1", {
      sdp: "local-sdp",
    });
    expect(setRemoteDescription).toHaveBeenCalledWith({
      type: "answer",
      sdp: "remote-sdp",
    });
    expect(screen.getByRole("link", { name: "Open session" })).toHaveAttribute(
      "href",
      "/sessions/session-9",
    );

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /hang up/i }));
    });
    await waitFor(() =>
      expect(mockEndSessionVoice).toHaveBeenCalledWith("session-9", "voice-9", "client_ended"),
    );
    expect(stopTrack).toHaveBeenCalled();
    expect(close).toHaveBeenCalled();
    expect(screen.getByText("Ended")).toBeInTheDocument();
  });

  it("shows the API error and returns to idle", async () => {
    mockStartChannelVoice.mockRejectedValue(new Error("Voice channel is disabled"));

    render(<VoiceTalkButton agentId="agent-1" channelId="appchan_1" />);
    fireEvent.click(screen.getByRole("button", { name: /talk to this channel/i }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Voice channel is disabled");
    expect(screen.getByRole("button", { name: /talk to this channel/i })).toBeInTheDocument();
    expect(mockEndSessionVoice).not.toHaveBeenCalled();
  });
});
