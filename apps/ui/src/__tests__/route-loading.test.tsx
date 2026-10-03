import { render, screen } from "@testing-library/react";
import MainLoading from "@/app/(main)/loading";
import SessionLoading from "@/app/(main)/sessions/[sessionId]/loading";

describe("route loading boundaries", () => {
  it.each([
    ["main", MainLoading],
    ["session", SessionLoading],
  ])("keeps the %s placeholder inside the content area", (_name, Loading) => {
    render(<Loading />);

    const status = screen.getByRole("status");
    expect(status).toHaveAttribute("aria-busy", "true");
    expect(status).toHaveTextContent("Loading");
    expect(status.className).not.toContain("h-screen");
    expect(status.className).not.toContain("fixed");
  });
});
