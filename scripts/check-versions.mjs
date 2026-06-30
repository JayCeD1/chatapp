#!/usr/bin/env node
// Keeps the app version in sync across the three places it lives, so a release can't
// ship mismatched versions. `package.json` is the source of truth.
//
//   node scripts/check-versions.mjs            # check (exit 1 on mismatch) — used by CI
//   node scripts/check-versions.mjs --write    # rewrite the others to match package.json
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const write = process.argv.includes("--write");

const pkgPath = join(root, "package.json");
const confPath = join(root, "src-tauri", "tauri.conf.json");
const cargoPath = join(root, "src-tauri", "Cargo.toml");

const pkgRaw = readFileSync(pkgPath, "utf8");
const confRaw = readFileSync(confPath, "utf8");
const cargoRaw = readFileSync(cargoPath, "utf8");

const source = JSON.parse(pkgRaw).version;
const confVersion = JSON.parse(confRaw).version;
// The [package] version starts at column 0; dependency versions are inline ({ version = … }).
const cargoVersion = cargoRaw.match(/^version\s*=\s*"([^"]+)"/m)?.[1];

const found = {
  "package.json": source,
  "src-tauri/tauri.conf.json": confVersion,
  "src-tauri/Cargo.toml": cargoVersion,
};

if (write) {
  if (confVersion !== source) {
    writeFileSync(
      confPath,
      confRaw.replace(/("version"\s*:\s*)"[^"]+"/, `$1"${source}"`),
    );
  }
  if (cargoVersion !== source) {
    writeFileSync(
      cargoPath,
      cargoRaw.replace(/^(version\s*=\s*)"[^"]+"/m, `$1"${source}"`),
    );
  }
  console.log(`✓ Synced tauri.conf.json and Cargo.toml to ${source}`);
  process.exit(0);
}

const ok = Object.values(found).every((v) => v === source);
if (!ok) {
  console.error(
    `✗ Version mismatch (package.json is the source of truth = ${source}):`,
  );
  for (const [file, v] of Object.entries(found)) {
    console.error(`    ${v === source ? "ok" : "!!"}  ${file}: ${v ?? "(not found)"}`);
  }
  console.error("\nRun `npm run version:sync` to align them, then commit.");
  process.exit(1);
}
console.log(`✓ All versions in sync: ${source}`);
