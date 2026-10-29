import { test } from "node:test";
import assert from "node:assert/strict";
import { buildSurfaces, chartWithBaseline, isLegacy, splitUIBlocks } from "../src/protocol.ts";
import { parseA2UI } from "../src/partial.ts";

const create = {
  version: "v1.0",
  createSurface: {
    surfaceId: "demo",
    catalogId: "copilotkit/standard@1",
    components: [{ id: "root", component: "Text", text: "Hello" }],
  },
};
const fence = (operations: unknown[], target = "") =>
  `\`\`\`a2ui${target}\n${operations.map((op) => JSON.stringify(op)).join("\n")}\n\`\`\``;
test("code examples containing widget fences remain code", () => {
  const text = "````markdown\n```a2ui\n{}\n```\n````\n";
  assert.equal(splitUIBlocks(text).filter((s) => s.type === "a2ui").length, 0);
  const openui = "````markdown\n```openui\nroot = Card([])\n```\n````\n";
  assert.equal(splitUIBlocks(openui).filter((s) => s.type === "openui").length, 0);
  assert.equal(
    splitUIBlocks(openui)
      .map((s) => s.content)
      .join(""),
    openui,
  );
  assert.equal(
    splitUIBlocks(text)
      .map((s) => s.content)
      .join(""),
    text,
  );
});
test("mixed markdown, OpenUI, A2UI and CRLF preserve source order", () => {
  const segments = splitUIBlocks(
    `Before\r\n\`\`\`openui\r\nroot = Card([])\r\n\`\`\`\r\n${fence([create])}\nAfter`,
  );
  assert.deepEqual(
    segments.filter((s) => s.type !== "markdown").map((s) => s.type),
    ["openui", "a2ui"],
  );
});
test("updates in another fence update the original surface without duplicating it", () => {
  const update = {
    version: "v1.0",
    updateDataModel: { surfaceId: "demo", path: "/total", value: 25 },
  };
  const result = buildSurfaces([{ id: "message", text: `${fence([create])}\n${fence([update])}` }]);
  assert.equal(result.surfaces.size, 1);
  assert.equal([...result.surfaces.values()][0].operations.length, 2);
  assert.equal(result.mounts.size, 1);
});
test("inline surfaces stay scoped to their reply; named targets span replies", () => {
  const messages = [
    { id: "a", text: fence([create]) },
    { id: "b", text: fence([create]) },
  ];
  assert.equal(buildSurfaces(messages).surfaces.size, 2);
  assert.equal(
    buildSurfaces(messages.map((m) => ({ ...m, text: fence([create], " target=preview") })))
      .surfaces.size,
    1,
  );
});
test("only complete operations are ingested during streaming", () => {
  const result = buildSurfaces(
    [
      {
        id: "a",
        text: `\`\`\`a2ui\n${JSON.stringify(create)}\n{"version":"v1.0","updateComponents":`,
      },
    ],
    true,
  );
  assert.equal(result.surfaces.size, 1);
  assert.equal([...result.surfaces.values()][0].status, "streaming");
});
test("malformed, oversized and unsupported blocks produce visible errors", () => {
  for (const text of [
    fence([{}]),
    "```a2ui\nnot json\n```",
    "```a2ui\n" + " ".repeat(256 * 1024 + 1) + "\n```",
  ]) {
    assert.equal(buildSurfaces([{ id: "a", text }]).errors.size, 1);
  }
});
test("legacy trees with arbitrary key order keep their renderer", () => {
  assert.equal(isLegacy('{"props":{},"type":"Card"}'), true);
  assert.equal(isLegacy(JSON.stringify(create)), false);
  assert.deepEqual(parseA2UI('{"type":"Card","children":['), { type: "Card", children: [] });
});
test("user messages never create surfaces", () => {
  assert.equal(
    buildSurfaces([{ id: "user", role: "user", text: fence([create]) }]).surfaces.size,
    0,
  );
});

test("bar charts use zero for nonnegative data without overriding authored or negative ranges", () => {
  assert.deepEqual(
    chartWithBaseline({
      data: [
        { week: "One", tickets: 5 },
        { week: "Two", tickets: 12 },
      ],
    }).yAxis,
    { min: 0 },
  );
  const negative = { data: [{ amount: -2 }, { amount: 5 }] };
  assert.equal(chartWithBaseline(negative), negative);
  const explicit = { data: [{ amount: 5 }], yAxis: { min: 4 } };
  assert.equal(chartWithBaseline(explicit), explicit);
  const line = { chartType: "line", data: [{ amount: 5 }] };
  assert.equal(chartWithBaseline(line), line);
});
