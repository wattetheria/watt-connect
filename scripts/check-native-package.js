#!/usr/bin/env node
"use strict";

// Check that a staged platform package contains its binary and, when it
// matches the current machine, that the binary runs.
// Usage: node scripts/check-native-package.js npm/native/linux-x64

const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { binaryName } = require("../npm/lib/native");

const dir = path.resolve(process.argv[2] || ".");
const pkg = JSON.parse(fs.readFileSync(path.join(dir, "package.json"), "utf8"));
const [platform] = pkg.os;
const [arch] = pkg.cpu;
const binary = path.join(dir, "bin", binaryName(platform));
if (!fs.existsSync(binary)) {
  console.error(`${pkg.name}: missing ${binary}`);
  process.exit(1);
}
if (platform === process.platform && arch === process.arch) {
  const result = spawnSync(binary, ["--version"], { encoding: "utf8" });
  if (result.status !== 0 || !result.stdout.includes("watt-connect")) {
    console.error(`${pkg.name}: ${binary} --version failed: ${result.stderr || result.error}`);
    process.exit(1);
  }
  console.log(`${pkg.name}: ${result.stdout.trim()}`);
} else {
  console.log(`${pkg.name}: binary present (not runnable on this machine)`);
}
