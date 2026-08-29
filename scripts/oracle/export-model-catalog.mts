/**
 * Exports the generated pi-ai model catalogs (models.generated.ts and
 * image-models.generated.ts) to JSON for the Rust port.
 *
 * The TypeScript generated data is the source of truth produced by
 * `scripts/generate-models.ts` upstream; this script evaluates it with the
 * TypeScript oracle and serializes it, giving the Rust embedding a
 * reproducible derivation (`node --experimental-strip-types
 * scripts/oracle/export-model-catalog.mts --write`).
 */

import { MODELS } from "../../pi-core/ai/src/models.generated.ts";
import { IMAGE_MODELS } from "../../pi-core/ai/src/image-models.generated.ts";

const write = process.argv.includes("--write");
const payload = JSON.stringify({ models: MODELS, imageModels: IMAGE_MODELS }, null, "\t");

if (write) {
  const { writeFileSync, mkdirSync } = await import("node:fs");
  mkdirSync(new URL("../../src/ai/data/", import.meta.url), { recursive: true });
  writeFileSync(
    new URL("../../src/ai/data/models.generated.json", import.meta.url),
    `${payload}\n`,
  );
  console.log("wrote src/ai/data/models.generated.json");
} else {
  console.log(payload.slice(0, 400));
}
