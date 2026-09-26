import path from "node:path";
import { fileURLToPath } from "node:url";

const desktopRoot = path.dirname(fileURLToPath(import.meta.url));
const appBinary = path.resolve(desktopRoot, "../../target/debug/nebula-desktop");

export const config = {
  runner: "local",
  specs: ["./test/webview.e2e.mjs"],
  maxInstances: 1,
  capabilities: [
    {
      browserName: "tauri",
      "tauri:options": { application: appBinary },
    },
  ],
  services: [
    ["@wdio/tauri-service", { appBinaryPath: appBinary, driverProvider: "embedded" }],
  ],
  framework: "mocha",
  mochaOpts: { ui: "bdd", timeout: 30_000 },
  waitforTimeout: 10_000,
  logLevel: "error",
};
