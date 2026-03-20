import { render, screen } from "@testing-library/react";
import { ScrollArea } from "@/components/ui/scroll-area";

describe("ScrollArea", () => {
  it("keeps viewport constrained when callers use max-height wrappers", () => {
    render(
      <ScrollArea className="max-h-[40vh]" data-testid="scroll-area">
        <div>content</div>
      </ScrollArea>,
    );

    const root = screen.getByTestId("scroll-area");
    const viewport = root.querySelector('[data-slot="scroll-area-viewport"]');

    expect(root).toHaveClass("overflow-hidden");
    expect(viewport).not.toBeNull();
    expect(viewport as HTMLElement).toHaveClass("[max-height:inherit]");
  });
});
