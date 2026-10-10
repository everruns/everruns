import { CHANNEL_DISABLE_HINT, channelPublishControl } from "@/lib/channel-display";

describe("channel publish and disable", () => {
  it("treats publish as the door and disable as a return to draft", () => {
    expect(channelPublishControl({ enabled: true, status: "draft" })).toMatchObject({
      label: "Draft",
      live: false,
      canPublish: true,
      hint: "Publish opens this channel to callers.",
    });
    expect(channelPublishControl({ enabled: true, status: "live" })).toMatchObject({
      label: "Published",
      live: true,
      canPublish: true,
      hint: "Unpublish closes this channel and leaves it ready to publish again.",
    });
    expect(channelPublishControl({ enabled: false, status: "disabled" })).toMatchObject({
      label: "Off",
      live: false,
      canPublish: false,
    });
    expect(CHANNEL_DISABLE_HINT).toMatch(/draft/);
    expect(CHANNEL_DISABLE_HINT).toMatch(/publish/i);
  });
});
