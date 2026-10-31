import { render, screen } from "@testing-library/react";
import { existsSync } from "node:fs";
import { join } from "node:path";
import SessionLoading from "@/app/(main)/sessions/[sessionId]/loading";

describe("route loading boundaries", () => {
  it("preserves the current page during navigation instead of replacing static chrome", () => {
    expect(existsSync(join(process.cwd(), "src/app/(main)/loading.tsx"))).toBe(false);
  });

  it("keeps the session placeholder inside the content area", () => {
    render(<SessionLoading />);

    const status = screen.getByRole("status");
    expect(status).toHaveAttribute("aria-busy", "true");
    expect(status).toHaveTextContent("Loading");
    expect(status.className).not.toContain("h-screen");
    expect(status.className).not.toContain("fixed");
  });
});
