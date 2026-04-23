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
  });

  it("shows the legacy GPT Image 1 description when configured", () => {
    render(
      <CapabilitySettingsEditor
        capabilityId="gpt_image_gen"
        config={{ model: "gpt-image-1" }}
        onChange={jest.fn()}
      />,
    );

    expect(screen.getByText("GPT Image 1")).toBeInTheDocument();
    expect(screen.getByText(/Legacy fallback/i)).toBeInTheDocument();
  });
});
