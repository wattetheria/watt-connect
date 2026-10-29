"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const { binaryName, packageName, resolveBinary } = require("../lib/native");

test("binary and package names", () => {
  assert.equal(binaryName("win32"), "watt-connect.exe");
  assert.equal(binaryName("darwin"), "watt-connect");
  assert.equal(packageName("linux-x64"), "@wattetheria/connect-linux-x64");
});

test("WATT_CONNECT_BINARY wins", () => {
  const file = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "wc-")), "watt-connect");
  fs.writeFileSync(file, "");
  assert.equal(resolveBinary({ env: { WATT_CONNECT_BINARY: file } }), file);
  assert.throws(() => resolveBinary({ env: { WATT_CONNECT_BINARY: `${file}-missing` } }), /missing file/);
});

test("resolves the platform package binary", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "wc-pkg-"));
  fs.mkdirSync(path.join(dir, "bin"));
  fs.writeFileSync(path.join(dir, "package.json"), "{}");
  fs.writeFileSync(path.join(dir, "bin", "watt-connect"), "");
  const found = resolveBinary({
    env: {},
    platform: "linux",
    arch: "x64",
    resolvePackage: (name) => {
      assert.equal(name, "@wattetheria/connect-linux-x64");
      return path.join(dir, "package.json");
    }
  });
  assert.equal(found, path.join(dir, "bin", "watt-connect"));
});

test("clear errors for unsupported or missing packages", () => {
  assert.throws(() => resolveBinary({ env: {}, platform: "freebsd", arch: "x64" }), /does not ship/);
  assert.throws(
    () =>
      resolveBinary({
        env: {},
        platform: "darwin",
        arch: "arm64",
        resolvePackage: () => {
          throw new Error("nope");
        }
      }),
    /optional dependency/
  );
});
