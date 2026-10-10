import { act, fireEvent, render, renderHook, screen } from "@testing-library/react";
import { useState } from "react";
import { useAgentDraft } from "@/components/agents/use-agent-draft";
import { CommunicationSelect } from "@/components/agents/communication-select";
import { normalizeCommunication } from "@/lib/agent-communication";
import type { Agent, Communication } from "@/lib/api/types";

function agent(overrides: Partial<Agent> = {}): Agent {
  return {
    id: "agent_1",
    name: "support",
    display_name: "Support",
    description: null,
    system_prompt: "Help people.",
    harness_id: "harness_1",
    default_model_id: null,
    tags: [],
    capabilities: [],
    status: "active",
    created_at: "2026-10-01T00:00:00Z",
    updated_at: "2026-10-01T00:00:00Z",
    archived_at: null,
    deleted_at: null,
    ...overrides,
  };
}

function build(result: { current: ReturnType<typeof useAgentDraft> }) {
  const built = result.current.buildRequest();
  if (!built.ok) throw new Error(JSON.stringify(built.errors));
  return built.request;
}

describe("agent Communication setting", () => {
  it("loads the saved value and defaults missing values to direct", () => {
    expect(renderHook(() => useAgentDraft(agent())).result.current.fields.communication).toBe(
      "direct",
    );
    expect(
      renderHook(() => useAgentDraft(agent({ communication: "explicit" }))).result.current.fields
        .communication,
    ).toBe("explicit");
    expect(normalizeCommunication(undefined)).toBe("direct");
    expect(normalizeCommunication("bogus")).toBe("direct");
  });

  it("leaves communication out of the update when untouched", () => {
    const { result } = renderHook(() => useAgentDraft(agent({ communication: "explicit" })));
    expect(result.current.isDirty).toBe(false);
    expect(build(result)).not.toHaveProperty("communication");
  });

  it("sends a changed value and round-trips it", () => {
    const saved = agent();
    const { result } = renderHook(() => useAgentDraft(saved));
    act(() => result.current.setField("communication", "explicit"));
    expect(result.current.isDirty).toBe(true);
    const request = build(result);
    expect(request.communication).toBe("explicit");

    // What the server returns after saving loads back as the same value.
    const reloaded = renderHook(() =>
      useAgentDraft({ ...saved, communication: request.communication }),
    );
    expect(reloaded.result.current.fields.communication).toBe("explicit");
    expect(build(reloaded.result)).not.toHaveProperty("communication");
  });

  it("offers Direct and Explicit with their descriptions", async () => {
    const onChange = jest.fn();
    function Harness() {
      const [value, setValue] = useState<Communication>("direct");
      return (
        <CommunicationSelect
          id="communication"
          value={value}
          onValueChange={(next) => {
            setValue(next);
            onChange(next);
          }}
        />
      );
    }
    render(
      <>
        <label htmlFor="communication">Communication</label>
        <Harness />
      </>,
    );

    expect(screen.getByLabelText("Communication")).toHaveTextContent("Direct");
    expect(screen.getByText("Replies are the agent's text, shown as it writes.")).toBeVisible();

    fireEvent.click(screen.getByLabelText("Communication"));
    const option = await screen.findByRole("option", { name: "Explicit" });
    fireEvent.pointerDown(option, { pointerType: "mouse" });
    fireEvent.click(option);

    expect(onChange).toHaveBeenLastCalledWith("explicit");
    expect(screen.getByLabelText("Communication")).toHaveTextContent("Explicit");
    expect(screen.getByText(/It talks only through send_message/)).toBeVisible();
  });
});
