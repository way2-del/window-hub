/**
 * Sync app version across package.json / tauri.conf.json / Cargo.toml.
 * Usage:
 *   node scripts/set-version.mjs              -> print current
 *   node scripts/set-version.mjs 0.1.1        -> set & print
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const VER_RE = /^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/;

function readJson(rel) {
  const p = path.join(root, rel);
  return { p, data: JSON.parse(fs.readFileSync(p, "utf8")) };
}

function writeJson(p, data) {
  fs.writeFileSync(p, `${JSON.stringify(data, null, 2)}\n`, "utf8");
}

function currentVersion() {
  const { data } = readJson("src-tauri/tauri.conf.json");
  return String(data.version || "");
}

function setVersion(ver) {
  if (!VER_RE.test(ver)) {
    console.error(`[ERROR] invalid version: ${ver} (expect x.y.z)`);
    process.exit(1);
  }

  const tauri = readJson("src-tauri/tauri.conf.json");
  tauri.data.version = ver;
  writeJson(tauri.p, tauri.data);

  const pkg = readJson("package.json");
  pkg.data.version = ver;
  writeJson(pkg.p, pkg.data);

  const cargoPath = path.join(root, "src-tauri/Cargo.toml");
  let cargo = fs.readFileSync(cargoPath, "utf8");
  // Only bump [package] version (first occurrence at crate root).
  let replaced = false;
  cargo = cargo.replace(/^version\s*=\s*"[^"]*"/m, (m) => {
    if (replaced) return m;
    replaced = true;
    return `version = "${ver}"`;
  });
  if (!replaced) {
    console.error("[ERROR] version = ... not found in src-tauri/Cargo.toml");
    process.exit(1);
  }
  fs.writeFileSync(cargoPath, cargo, "utf8");

  console.log(ver);
}

const arg = (process.argv[2] || "").trim();
if (!arg || arg === "--get") {
  const v = currentVersion();
  if (!v) {
    console.error("[ERROR] version missing in tauri.conf.json");
    process.exit(1);
  }
  console.log(v);
  process.exit(0);
}

setVersion(arg);
