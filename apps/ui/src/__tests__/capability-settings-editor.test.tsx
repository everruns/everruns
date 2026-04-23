import { render, screen } from "@testing-library/react";
import {
  CapabilitySettingsEditor,
  hasCapabilitySettings,
} from "@/components/agents/capability-settings-editor";

describe("CapabilitySettingsEditor", () => {
  it("exposes settings for OpenAI image generation", () => {
    expect(hasCapabilitySettings("gpt_image_gen")).toBe(true);
  });

  it("shows ChatGPT Images 2.0 as the default image model", () => {
    render(
      <CapabilitySettingsEditor capabilityId="gpt_image_gen" config={{}} onChange={jest.fn()} />,
    );

    expect(screen.getByText("ChatGPT Images 2.0")).toBeInTheDocument();
    expect(
      screen.getByText(/Uses OpenAI's current ChatGPT Images 2.0 API model/i),
    ).toBeInTheDocument();
    expect(screen.getByText("Medium")).toBeInTheDocument();
    expect(screen.getByText(/Balanced quality and latency/i)).toBeInTheDocument();
  });

  it("shows the configured legacy model and high-quality descriptions", () => {
    render(
      <CapabilitySettingsEditor
        capabilityId="gpt_image_gen"
        config={{ model: "gpt-image-1", default_quality: "high" }}
        onChange={jest.fn()}
      />,
    );

    expect(screen.getByText("GPT Image 1")).toBeInTheDocument();
    expect(screen.getByText(/Legacy fallback/i)).toBeInTheDocument();
    expect(screen.getByText(/Highest fidelity, but materially slower/i)).toBeInTheDocument();
  });
});
