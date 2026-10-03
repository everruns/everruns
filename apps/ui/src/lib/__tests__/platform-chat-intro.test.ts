import { resolvePlatformChatIntro } from "../platform-chat-intro";
it("uses presentation from any Agent", () => {
  expect(
    resolvePlatformChatIntro({
      intro_markdown: "Hello",
      short_description: "Support",
      starters: [{ text: "Help me" }],
    }),
  ).toEqual({ intro: "Hello", description: "Support", starters: [{ text: "Help me" }] });
});
it("does not supply presentation for an agentless conversation", () => {
  expect(resolvePlatformChatIntro(undefined)).toEqual({
    intro: null,
    description: null,
    starters: [],
  });
});
it("allows an Agent to have no intro or starters", () => {
  expect(resolvePlatformChatIntro({ intro_markdown: "", starters: [] })).toEqual({
    intro: null,
    description: null,
    starters: [],
  });
});
