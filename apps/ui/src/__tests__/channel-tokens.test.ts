import { DEFAULT_CHANNEL_TOKEN_LENGTH, generateChannelToken } from "@/lib/channel-tokens";

describe("generateChannelToken", () => {
  it("generates 63-character URL-safe tokens by default", () => {
    const token = generateChannelToken();

    expect(token).toHaveLength(DEFAULT_CHANNEL_TOKEN_LENGTH);
    expect(token).toMatch(/^[A-Za-z0-9_-]+$/);
  });
});
