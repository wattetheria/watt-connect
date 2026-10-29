"use strict";

const fs = require("node:fs");
const path = require("node:path");

const BASE_NAME = "watt-connect";
const PACKAGE_PREFIX = "@wattetheria/connect";
const SUPPORTED = new Set([
  "darwin-arm64",
  "darwin-x64",
  "linux-arm64",
  "linux-x64",
  "win32-arm64",
  "win32-x64"
]);

function binaryName(platform = process.platform) {
  return platform === "win32" ? `${BASE_NAME}.exe` : BASE_NAME;
}

function targetKey(platform = process.platform, arch = process.arch) {
  return `${platform}-${arch}`;
}

function packageName(key) {
  return `${PACKAGE_PREFIX}-${key}`;
}

/**
 * Locate the native binary:
 * 1. WATT_CONNECT_BINARY (development / custom builds),
 * 2. the matching optional dependency `@wattetheria/connect-<platform>-<arch>`.
 */
function resolveBinary({
  env = process.env,
  platform = process.platform,
  arch = process.arch,
  resolvePackage = (name) => require.resolve(`${name}/package.json`)
} = {}) {
  if (env.WATT_CONNECT_BINARY) {
    if (!fs.existsSync(env.WATT_CONNECT_BINARY)) {
      throw new Error(`WATT_CONNECT_BINARY points to a missing file: ${env.WATT_CONNECT_BINARY}`);
    }
    return env.WATT_CONNECT_BINARY;
  }
  const key = targetKey(platform, arch);
  if (!SUPPORTED.has(key)) {
    throw new Error(
      `watt-connect does not ship a binary for ${key}. Supported: ${[...SUPPORTED].join(", ")}.`
    );
  }
  const name = packageName(key);
  let manifest;
  try {
    manifest = resolvePackage(name);
  } catch {
    throw new Error(
      [
        `The native package ${name} is not installed.`,
        "It is an optional dependency of watt-connect; reinstall without --no-optional",
        "(or --omit=optional), or set WATT_CONNECT_BINARY to a watt-connect binary."
      ].join("\n")
    );
  }
  const binary = path.join(path.dirname(manifest), "bin", binaryName(platform));
  if (!fs.existsSync(binary)) {
    throw new Error(`${name} is installed but ${binary} is missing.`);
  }
  return binary;
}

module.exports = { BASE_NAME, PACKAGE_PREFIX, SUPPORTED, binaryName, packageName, resolveBinary, targetKey };
