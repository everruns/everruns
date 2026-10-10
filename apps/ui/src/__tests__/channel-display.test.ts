import { channelPublishControl } from "@/lib/channel-display";

describe("channel publish", () => {
  it("opens a closed channel in one step, including one stored as disabled", () => {
    expect(channelPublishControl({ enabled: true, status: "draft" })).toMatchObject({
      label: "Draft",
      live: false,
      hint: "Publish opens this channel to callers.",
    });
    expect(channelPublishControl({ enabled: false, status: "disabled" })).toMatchObject({
      label: "Draft",
      live: false,
      hint: "Publish opens this channel to callers.",
    });
    expect(channelPublishControl({ enabled: true, status: "live" })).toMatchObject({
      label: "Published",
      live: true,
      hint: "Unpublish closes this channel. Publish again to open it.",
    });
  });
});
