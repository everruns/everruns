// Publishes the OpenAPI schema as a static file at /api/openapi.json.
//
// starlight-openapi reads ../../docs/api/openapi.json at build time to generate
// the browsable reference, but never emits the spec itself. The generated
// reference pages are excluded from the llms-*.txt outputs precisely because the
// spec is the better machine-readable form — so the spec has to actually be
// served. A deployment serves it at /api-doc/openapi.json; the docs site had no
// copy at all and /api/openapi.json returned 404.
import { copyFileSync, mkdirSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const source = join(here, "../../../docs/api/openapi.json");
const destination = join(here, "../public/api/openapi.json");

mkdirSync(dirname(destination), { recursive: true });
copyFileSync(source, destination);

const { size } = statSync(destination);
console.log(
  `[copy-openapi] public/api/openapi.json written (${Math.round(size / 1024)} KB)`
);
