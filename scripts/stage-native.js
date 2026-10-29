#!/usr/bin/env node
"use strict";

// Copy a built watt-connect binary into npm/native/<platform>-<arch>/bin.
// Usage: node scripts/stage-native.js --platform linux --arch x64 --source target/release/watt-connect

const fs = require("node:fs");
const path = require("node:path");
const { SUPPORTED, binaryName } = require("../npm/lib/native");

const ROOT = path.resolve(__dirname, "..");

function parseArgs(argv) {
  const options = { platform: process.platform, arch: process.arch, source: "" };
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    const value = argv[i + 1];
    if (!["--platform", "--arch", "--source"].includes(flag) || !value || value.startsWith("--")) {
      throw new Error(`Unknown or incomplete option: ${flag}`);
    }
    options[flag.slice(2)] = value;
    i += 1;
  }
  return options;
}

function stage({ platform, arch, source }) {
  const key = `${platform}-${arch}`;
  if (!SUPPORTED.has(key)) {
    throw new Error(`Unsupported target ${key}`);
  }
  const from = path.resolve(ROOT, source || path.join("target", "release", binaryName(platform)));
  if (!fs.existsSync(from)) {
    throw new Error(`Binary not found: ${from}`);
  }
  const binDir = path.join(ROOT, "npm", "native", key, "bin");
  fs.mkdirSync(binDir, { recursive: true });
  const to = path.join(binDir, binaryName(platform));
  fs.copyFileSync(from, to);
  if (platform !== "win32") {
    fs.chmodSync(to, 0o755);
  }
  return to;
}

if (require.main === module) {
  try {
    console.log(`staged ${stage(parseArgs(process.argv.slice(2)))}`);
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}

module.exports = { parseArgs, stage };
