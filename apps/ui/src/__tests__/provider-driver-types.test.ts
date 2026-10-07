import { isDriverOffered } from "@/lib/api/provider-driver-types";

describe("isDriverOffered", () => {
  it("always offers ungated drivers", () => {
    expect(isDriverOffered("openai", undefined)).toBe(true);
    expect(isDriverOffered("anthropic", [])).toBe(true);
  });

  it("offers a flag-gated driver only once the server config lists it", () => {
    expect(isDriverOffered("mistral", undefined)).toBe(false);
    expect(isDriverOffered("mistral", [{ driver: "openai" }])).toBe(false);
    expect(isDriverOffered("mistral", [{ driver: "mistral" }])).toBe(true);
    expect(isDriverOffered("chatgpt", [{ driver: "mistral" }])).toBe(false);
  });
});
