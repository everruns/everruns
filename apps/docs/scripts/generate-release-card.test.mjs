import assert from "node:assert/strict";
import test from "node:test";

import {
  checkRenderedWidths,
  releaseCardSvg,
  validateChangelog,
  validateReleaseCard,
} from "./generate-release-card.mjs";

const validCard = {
  version: "1.2.3",
  date: "2026-09-04",
  headline: ["Reasoning,", "in order."],
  summary: ["A concise release summary."],
  highlights: [
    {
      title: "Ordered artifacts",
      description: "Replay work in the order it happened.",
      source: "#123",
    },
  ],
};

test("validates and normalizes release card content", () => {
  assert.deepEqual(
    validateReleaseCard(
      {
        ...validCard,
        version: " 1.2.3 ",
        date: " 2026-09-04 ",
        headline: [" Reasoning, ", " in order. "],
        summary: [" A concise release summary. "],
        highlights: [
          {
            title: " Ordered artifacts ",
            description: " Replay work in the order it happened. ",
            source: " #123 ",
          },
        ],
        ignored: "not part of the card",
      },
      "1.2.3",
    ),
    validCard,
  );
});

test("rejects a card for a different release", () => {
  assert.throws(() => validateReleaseCard(validCard, "1.2.4"), /does not match release 1\.2\.4/);
});

test("rejects content that cannot fit the template", () => {
  assert.deepEqual(validateReleaseCard({ ...validCard, headline: ["x".repeat(24)] }).headline, [
    "x".repeat(24),
  ]);
  assert.throws(
    () => validateReleaseCard({ ...validCard, headline: ["x".repeat(25)] }),
    /headline\[0\] must be at most 24 characters/,
  );
  assert.throws(
    () =>
      validateReleaseCard({
        ...validCard,
        highlights: [
          {
            ...validCard.highlights[0],
            description: "x ".repeat(49).trim(),
          },
        ],
      }),
    /description must fit on two lines/,
  );
});

test("rejects impossible calendar dates", () => {
  assert.throws(
    () => validateReleaseCard({ ...validCard, date: "2026-02-31" }),
    /valid YYYY-MM-DD date/,
  );
});

test("requires highlight sources in the matching changelog section", () => {
  const card = validateReleaseCard(validCard);
  validateChangelog(card, "## [1.2.3] - 2026-09-04\n\n- Ordered artifacts ([#123](example))\n");
  assert.throws(
    () => validateChangelog(card, "## [1.2.3] - 2026-09-04\n\n- Something else\n"),
    /source #123 is not present/,
  );
  for (const changelog of [
    "## [1.2.4] - 2026-09-05\n\n- #123\n\n## [1.2.3] - 2026-09-04\n\n- Something else\n",
    "## [1.2.3] - 2026-09-04\n\n- Something else\n\n## [1.2.2] - 2026-09-03\n\n- #123\n",
  ]) {
    assert.throws(() => validateChangelog(card, changelog), /source #123 is not present/);
  }
});

test("escapes user-controlled SVG text", () => {
  const svg = releaseCardSvg({
    ...validCard,
    highlights: [{ title: "Files & tools", description: "Use <guarded> input.", source: "#123" }],
  });
  assert.match(svg, /Files &amp; tools/);
  assert.match(svg, /Use &lt;guarded&gt; input\./);
  assert.doesNotMatch(svg, /Use <guarded>/);
});

test("accepts up to four highlights", () => {
  const item = validCard.highlights[0];
  assert.equal(validateReleaseCard({ ...validCard, highlights: [item, item, item, item] }).highlights.length, 4);
  assert.throws(
    () => validateReleaseCard({ ...validCard, highlights: [item, item, item, item, item] }),
    /highlights must contain 1-4 items/,
  );
});

test("rejects a headline that renders wider than its column", async () => {
  const { default: sharp } = await import("sharp");
  await checkRenderedWidths(validateReleaseCard({ ...validCard, headline: ["Actor model,", "Agent API."] }), sharp);
  // 20 characters, under the character limit, yet wider than the headline column.
  await assert.rejects(
    checkRenderedWidths(validateReleaseCard({ ...validCard, headline: ["Agents you can call,"] }), sharp),
    /headline "Agents you can call," renders \d+px wide/,
  );
});
