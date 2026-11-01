import { readFileSync } from "node:fs";
import { join } from "node:path";
import { GRADER_GROUPS } from "@/components/evals/section/graders-list";

// The Graders tab must describe every grader the API accepts.
it("describes every scorer variant in the API", () => {
  const spec = JSON.parse(
    readFileSync(join(__dirname, "../../../../docs/api/openapi.json"), "utf8"),
  ) as {
    components: {
      schemas: { Scorer: { oneOf: { properties: { type: { enum: string[] } } }[] } };
    };
  };
  const api = spec.components.schemas.Scorer.oneOf.map((v) => v.properties.type.enum[0]).sort();
  const listed = GRADER_GROUPS.flatMap((g) => g.graders.map((x) => x.type)).sort();
  expect(listed).toEqual(api);
});
