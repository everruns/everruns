import { pickMcpIconSrc, safeMcpIconSrc } from "@/components/connections/mcp-server-mark";

const png = "data:image/png;base64,aaaa";

describe("mcp server marks", () => {
  it("keeps https and raster data icons and drops the rest", () => {
    expect(safeMcpIconSrc("https://mcp.example.com/icon.png")).toBe(
      "https://mcp.example.com/icon.png",
    );
    expect(safeMcpIconSrc(png)).toBe(png);
    expect(safeMcpIconSrc("http://mcp.example.com/icon.png")).toBeNull();
    expect(safeMcpIconSrc("https://user:pass@mcp.example.com/icon.png")).toBeNull();
    expect(safeMcpIconSrc("data:image/svg+xml;base64,aaaa")).toBeNull();
    expect(safeMcpIconSrc("javascript:alert(1)")).toBeNull();
  });

  it("prefers the icon for the current theme", () => {
    const icons = [
      { src: "https://mcp.example.com/light.png", theme: "light" as const },
      { src: "https://mcp.example.com/dark.png", theme: "dark" as const },
    ];
    expect(pickMcpIconSrc(icons, "dark", png)).toBe("https://mcp.example.com/dark.png");
    expect(pickMcpIconSrc(icons, "light")).toBe("https://mcp.example.com/light.png");
    expect(pickMcpIconSrc([], "light", png)).toBe(png);
  });
});
