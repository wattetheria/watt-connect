#!/usr/bin/env node
"use strict";

// Thin launcher: find the platform binary and run it with inherited stdio, so
// the MCP stdio stream goes straight between the agent and the native process.

const { spawn } = require("node:child_process");
const { resolveBinary } = require("../lib/native");

let binary;
try {
  binary = resolveBinary();
} catch (error) {
  console.error(error.message);
  process.exit(1);
}

const child = spawn(binary, process.argv.slice(2), { stdio: "inherit", windowsHide: true });

for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  process.on(signal, () => {
    if (!child.killed) {
      child.kill(signal);
    }
  });
}

child.on("error", (error) => {
  console.error(`watt-connect: failed to start ${binary}: ${error.message}`);
  process.exit(1);
});

child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }
  process.exit(code === null ? 1 : code);
});
