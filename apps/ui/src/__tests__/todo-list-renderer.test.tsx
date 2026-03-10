import { render, screen } from "@testing-library/react";
import { TodoListRenderer } from "@/components/chat/todo-list-renderer";

describe("TodoListRenderer", () => {
  it("renders a progress header and uses activeForm for in-progress items", () => {
    const { container } = render(
      <TodoListRenderer
        arguments={{
          todos: [
            {
              content: "Review code changes",
              activeForm: "Reviewing code changes",
              status: "completed",
            },
            {
              content: "Run tests",
              activeForm: "Running tests",
              status: "in_progress",
            },
            {
              content: "Update docs",
              activeForm: "Updating docs",
              status: "pending",
            },
          ],
        }}
        isExecuting
      />,
    );

    const root = container.firstElementChild as HTMLElement;

    expect(screen.getByText("1 of 3 todos completed")).toBeInTheDocument();
    expect(screen.getAllByText("Running tests")).toHaveLength(2);
    expect(screen.getByText("Update docs")).toBeInTheDocument();
    expect(root).not.toHaveClass("border");
    expect(root.className).not.toContain("bg-card/95");
  });
});
