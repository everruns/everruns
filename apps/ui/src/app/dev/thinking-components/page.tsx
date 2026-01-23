"use client";

/**
 * Dev page showcasing the StreamingThinking component
 */

import { StreamingThinking } from "@/components/streaming-thinking";

const SAMPLE_THINKING = `Let me analyze this step by step.

1. First, I need to understand the context of the question. The user is asking about why the sky appears blue.

2. This involves physics - specifically, the interaction of light with Earth's atmosphere.

3. The key concept here is Rayleigh scattering:
   - Sunlight contains all colors of the visible spectrum
   - As light enters the atmosphere, it collides with gas molecules (N2, O2)
   - Shorter wavelengths (blue, violet) scatter more than longer wavelengths (red, orange)
   - The scattering intensity is proportional to 1/wavelength^4

4. Why blue specifically and not violet (which scatters even more)?
   - The sun emits less violet light than blue
   - Our eyes are more sensitive to blue light
   - Some violet light is absorbed in the upper atmosphere

5. This explains related phenomena too:
   - Sunsets are red/orange (light travels through more atmosphere)
   - The sky near the horizon is paler`;

const SHORT_THINKING = "This is a simple arithmetic question. 2+2 equals 4.";

export default function ThinkingComponentsPage() {
  return (
    <div className="container mx-auto p-8 max-w-4xl space-y-8">
      <h1 className="text-3xl font-bold mb-2">Thinking Components</h1>
      <p className="text-muted-foreground mb-8">
        Showcase of the StreamingThinking component for extended thinking models
      </p>

      {/* Expanded state */}
      <section className="space-y-4">
        <h2 className="text-xl font-semibold">Expanded Thinking</h2>
        <p className="text-sm text-muted-foreground">
          Shows the model&apos;s chain-of-thought reasoning before producing the final response.
        </p>
        <StreamingThinking text={SAMPLE_THINKING} defaultCollapsed={false} />
      </section>

      {/* Collapsed state */}
      <section className="space-y-4">
        <h2 className="text-xl font-semibold">Collapsed Thinking</h2>
        <p className="text-sm text-muted-foreground">
          Users can collapse the thinking section to focus on the response.
        </p>
        <StreamingThinking text={SAMPLE_THINKING} defaultCollapsed={true} />
      </section>

      {/* Streaming state */}
      <section className="space-y-4">
        <h2 className="text-xl font-semibold">Streaming Thinking</h2>
        <p className="text-sm text-muted-foreground">
          Shows a pulsing indicator and cursor while the model is still thinking.
        </p>
        <StreamingThinking text={SHORT_THINKING} isStreaming={true} />
      </section>

      {/* In context example */}
      <section className="space-y-4">
        <h2 className="text-xl font-semibold">In Chat Context</h2>
        <p className="text-sm text-muted-foreground">
          Example of how thinking appears in a chat conversation.
        </p>
        <div className="border rounded-lg p-4 space-y-4 bg-background">
          {/* User message */}
          <div className="flex justify-end">
            <div className="bg-primary text-primary-foreground rounded-lg px-4 py-2 max-w-[80%]">
              Why is the sky blue?
            </div>
          </div>

          {/* Agent thinking + response */}
          <div className="space-y-3">
            <StreamingThinking text={SAMPLE_THINKING} defaultCollapsed={false} />
            <div className="text-sm prose prose-sm max-w-none">
              <p>
                The blue color of the sky is caused by <strong>Rayleigh scattering</strong>.
              </p>
              <p>
                When sunlight enters Earth&apos;s atmosphere, it collides with gas molecules.
                Shorter wavelengths (blue light) scatter more than longer wavelengths (red light),
                with scattering intensity proportional to 1/wavelength^4.
              </p>
              <p>
                We see blue rather than violet (which scatters even more) because:
              </p>
              <ul>
                <li>The sun emits less violet light than blue</li>
                <li>Our eyes are more sensitive to blue</li>
                <li>Some violet is absorbed in the upper atmosphere</li>
              </ul>
            </div>
          </div>
        </div>
      </section>
    </div>
  );
}
