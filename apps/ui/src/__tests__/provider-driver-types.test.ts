import { isDriverOffered, isPlatformProviderVisible } from "@/lib/api/provider-driver-types";

describe("isDriverOffered", () => {
  it("offers every platform driver, Mistral included", () => {
    expect(isDriverOffered("openai")).toBe(true);
    expect(isDriverOffered("anthropic")).toBe(true);
    expect(isDriverOffered("mistral")).toBe(true);
  });

  it("keeps the framework ChatGPT driver out of platform provider and model UI", () => {
    expect(isPlatformProviderVisible("openai")).toBe(true);
    expect(isPlatformProviderVisible("chatgpt")).toBe(false);
    expect(isDriverOffered("chatgpt")).toBe(false);
  });
});
