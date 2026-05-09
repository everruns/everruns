import { fireEvent, render, screen } from "@testing-library/react";
import { SkillDetailView } from "@/components/skills/skill-detail-view";
import type { Skill, SkillContent } from "@/lib/api/types";

jest.mock("streamdown", () => ({
  Streamdown: ({ children }: { children: string }) => (
    <pre data-testid="streamdown-mock">{children}</pre>
  ),
}));

jest.mock("@streamdown/code", () => ({
  code: {},
}));

jest.mock("@/components/chat/streamdown-message", () => ({
  InlineStreamdownMessage: ({ children }: { children: string }) => (
    <pre data-testid="inline-streamdown-mock">{children}</pre>
  ),
}));

const skill: Skill = {
  id: "skill_123",
  name: "data-analysis",
  description: "Analyze data files.",
  metadata: {},
  source_type: "archive",
  status: "active",
  version: "1",
  created_at: "2026-01-01T00:00:00.000Z",
  updated_at: "2026-01-02T00:00:00.000Z",
  archived_at: null,
  deleted_at: null,
};

const content: SkillContent = {
  skill_md:
    "---\nname: data-analysis\ndescription: Analyze data files.\n---\n\n# Data Analysis\n\nUse the scripts.",
  files: [{ path: "scripts/run.py", content: "print('ready')\n" }],
};

describe("SkillDetailView", () => {
  it("renders a read-only file browser tab with SKILL.md and extracted files", () => {
    render(
      <SkillDetailView
        skill={skill}
        usage={{ agents: 1, harnesses: 0 }}
        content={content}
        contentLoading={false}
        contentError={null}
      />,
    );

    fireEvent.click(screen.getByRole("tab", { name: /files/i }));

    expect(screen.getByText("Skill Files")).toBeTruthy();
    expect(screen.getByRole("button", { name: /SKILL\.md/i })).toBeTruthy();
    expect(screen.getByRole("button", { name: /scripts\/run\.py/i })).toBeTruthy();
    expect(screen.getAllByText("read-only")).toHaveLength(2);

    fireEvent.click(screen.getByRole("button", { name: /scripts\/run\.py/i }));

    expect(screen.getByText(/print\('ready'\)/)).toBeTruthy();
  });
});
