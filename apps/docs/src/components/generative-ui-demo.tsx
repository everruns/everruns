import { createRoot } from "react-dom/client";
import { useState } from "react";
import { GenerativeUIConversation, MessageWidgets } from "@everruns/generative-ui";
import samples from "../../../ui/generative-ui/tests/standard-samples.json";
import { BASE } from "../../site.config.mjs";

const index = Number(new URLSearchParams(location.search).get("example"));
const sample = Number.isInteger(index) ? samples[index] : undefined;

function Demo() {
  const [revision, setRevision] = useState(0);
  const [result, setResult] = useState("");
  if (!sample) return <p>Example not found.</p>;
  const text =
    "```a2ui\n" +
    JSON.stringify({
      version: "v1.0",
      createSurface: {
        surfaceId: `example-${index}-${revision}`,
        catalogId: "copilotkit/standard@1",
        dataModel: sample.data,
        components: sample.components,
      },
    }) +
    "\n```";
  return (
    <>
      <div className="demo-actions">
        <span>Interactive example · actions stay in this preview</span>
        <button
          onClick={() => {
            setResult("");
            setRevision(revision + 1);
          }}
        >
          Reset
        </button>
      </div>
      <GenerativeUIConversation
        key={revision}
        messages={[{ id: "demo", text }]}
        runtimeUrl={`${BASE}/generative-ui`}
        onMessage={setResult}
      >
        <MessageWidgets
          messageId="demo"
          text={text}
          onMessage={setResult}
          renderMarkdown={(value) => <p>{value}</p>}
        />
      </GenerativeUIConversation>
      {result && (
        <output className="demo-result" aria-label="Example action" aria-live="polite">
          {result}
        </output>
      )}
    </>
  );
}

const host = window.frameElement?.ownerDocument.documentElement;
const syncTheme = () => {
  const theme = host?.dataset.theme ?? "light";
  document.documentElement.dataset.theme = theme;
  document.documentElement.classList.toggle("dark", theme === "dark");
};
syncTheme();
const observer = new MutationObserver(syncTheme);
if (host) observer.observe(host, { attributes: true, attributeFilter: ["data-theme"] });
addEventListener("pagehide", () => observer.disconnect(), { once: true });
createRoot(document.getElementById("example")!).render(<Demo />);

// Size the embed from its content; long examples keep their own scroll region.
const resize = new ResizeObserver(() => {
  const frame = window.frameElement as HTMLIFrameElement | null;
  const minimum = sample?.covers.some((name) => name === "Modal" || name === "Popover") ? 420 : 180;
  if (frame)
    frame.style.height = `${Math.min(1000, Math.max(minimum, document.body.scrollHeight))}px`;
});
resize.observe(document.body);
addEventListener("pagehide", () => resize.disconnect(), { once: true });
