#!/usr/bin/env node
"use strict";

// Set one version on the main npm package, its optionalDependencies and every
// platform package. Usage: node scripts/set-version.js 0.2.0

const fs = require("node:fs");
const path = require("node:path");

const ROOT = path.resolve(__dirname, "..");
const version = process.argv[2];
if (!version || !/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) {
  console.error("usage: node scripts/set-version.js <semver>");
  process.exit(1);
}

function update(file, change) {
  const json = JSON.parse(fs.readFileSync(file, "utf8"));
  change(json);
  fs.writeFileSync(file, `${JSON.stringify(json, null, 2)}\n`);
  console.log(`${path.relative(ROOT, file)} -> ${version}`);
}

update(path.join(ROOT, "npm", "package.json"), (pkg) => {
  pkg.version = version;
  for (const name of Object.keys(pkg.optionalDependencies || {})) {
    pkg.optionalDependencies[name] = version;
  }
});
const nativeDir = path.join(ROOT, "npm", "native");
for (const key of fs.readdirSync(nativeDir)) {
  update(path.join(nativeDir, key, "package.json"), (pkg) => {
    pkg.version = version;
  });
}
