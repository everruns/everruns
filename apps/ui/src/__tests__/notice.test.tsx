import { render, screen } from "@testing-library/react";
import { ShieldAlert } from "lucide-react";

import { Notice, NoticeDescription, NoticeTitle } from "@/components/ui/notice";

describe("Notice", () => {
  it("renders a warning notice with shared structure", () => {
    render(
      <Notice
        variant="warning"
        icon={<ShieldAlert className="h-5 w-5" data-testid="notice-icon" aria-hidden="true" />}
      >
        <NoticeTitle>Full account access</NoticeTitle>
        <NoticeDescription>Treat API keys like passwords.</NoticeDescription>
      </Notice>,
    );

    const notice = screen.getByText("Full account access").closest('[data-slot="notice"]');

    expect(notice).toHaveAttribute("data-variant", "warning");
    expect(screen.getByTestId("notice-icon")).toBeInTheDocument();
    expect(screen.getByText("Treat API keys like passwords.")).toHaveAttribute(
      "data-slot",
      "notice-description",
    );
  });
});
