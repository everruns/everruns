import { render, screen } from "@testing-library/react";
import { Avatar, AvatarFallback } from "@/components/ui/avatar";

describe("Avatar", () => {
  it("uses Slate's sharp-cornered shape by default", () => {
    const { container } = render(
      <Avatar>
        <AvatarFallback>ER</AvatarFallback>
      </Avatar>,
    );

    expect(container.querySelector('[data-slot="avatar"]')).not.toHaveClass("rounded-full");
    expect(screen.getByText("ER")).not.toHaveClass("rounded-full");
  });
});
