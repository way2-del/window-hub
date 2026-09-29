/**
 * Bundle Bloub entries into the runtime plugin package.
 * Usage: node plugins/cozmo/build.mjs
 */
import { build } from "esbuild";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "../..");
const outDir = join(root, "src-tauri/resources/plugins/cozmo");
const exampleDir = join(root, "docs/plugins/examples/cozmo");

mkdirSync(outDir, { recursive: true });

const entries = [
  ["src/shortcuts-entry.ts", "shortcuts.js"],
  ["src/popup-entry.ts", "popup.js"],
];

for (const [entry, out] of entries) {
  await build({
    entryPoints: [join(root, "plugins/cozmo", entry)],
    bundle: true,
    format: "iife",
    outfile: join(outDir, out),
    minify: true,
    target: ["es2020"],
  });
  console.log("built", out);
}

// Keep examples in sync if present (plugins:sync will also copy)
try {
  mkdirSync(exampleDir, { recursive: true });
  for (const [, out] of entries) {
    copyFileSync(join(outDir, out), join(exampleDir, out));
  }
} catch {
  /* optional */
}
